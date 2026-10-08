//! Email + password auth (self-hosted default): `POST /api/auth/login`
//! verifies an Argon2id hash, `POST /api/auth/register` creates the user
//! behind the allowlist. Sessions are JWT cookies — no mail, no OAuth.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::json;

use runway_core::models::{Team, User};
use runway_core::slugify::{slugify, token_hex};

use runway_core::access::is_email_allowed;

use crate::auth::{mint_session, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Fire the access-denied webhook (best effort — devpush `notify_denied`).
pub(crate) async fn notify_denied(state: &AppState, email: &str, provider: &str) {
    let Some(url) = &state.settings.access_denied_webhook else {
        return;
    };
    let _ = reqwest::Client::new()
        .post(url)
        .json(&json!({ "email": email, "provider": provider }))
        .send()
        .await;
}

pub(crate) fn session_cookie(state: &AppState, user_id: i64) -> ApiResult<Cookie<'static>> {
    let jwt = mint_session(
        &state.settings.secret_key,
        user_id,
        state.settings.session_max_age,
    )
    .map_err(ApiError::internal)?;
    Ok(Cookie::build((state.settings.session_cookie.clone(), jwt))
        .path("/")
        .http_only(true)
        .secure(state.settings.url_scheme == "https")
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .max_age(time::Duration::seconds(
            state.settings.session_max_age as i64,
        ))
        .build())
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
}

/// `POST /api/auth/login` — email + password → session cookie.
/// Generic error either way to avoid account enumeration.
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Json(body): axum::Json<LoginBody>,
) -> ApiResult<Response> {
    let email = body.email.trim().to_lowercase();
    let user: Option<User> =
        sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1 AND status = 'active'")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
    let valid = user
        .as_ref()
        .and_then(|u| u.password_hash.as_deref())
        .map(|h| runway_core::password::verify(h, &body.password))
        .unwrap_or_else(|| {
            // Burn a verify anyway so missing users don't time differently.
            let _ = runway_core::password::hash(&body.password);
            false
        });
    if !valid {
        return Err(ApiError::unauthorized("invalid email or password"));
    }
    let user = ensure_personal_team(&state, user.unwrap()).await?;
    let jar = jar.add(session_cookie(&state, user.id)?);
    Ok((jar, axum::Json(json!({ "ok": true }))).into_response())
}

#[derive(Deserialize)]
pub struct RegisterBody {
    email: String,
    username: Option<String>,
    password: String,
}

/// `POST /api/auth/register` — create account + personal team + session.
/// Gated by the sign-up allowlist (empty = open, matching OIDC semantics).
pub async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Json(body): axum::Json<RegisterBody>,
) -> ApiResult<Response> {
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 320 {
        return Err(ApiError::bad_request("invalid email"));
    }
    if body.password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    if !is_email_allowed(&state.db, &email).await? {
        notify_denied(&state, &email, "email").await;
        return Err(ApiError::forbidden(
            state.settings.access_denied_message.clone(),
        ));
    }
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM \"user\" WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_some() {
        return Err(ApiError::bad_request("account already exists"));
    }
    let local = email.split('@').next().unwrap_or("user");
    let username = match &body.username {
        Some(u) if !u.trim().is_empty() => {
            let s = slugify(u.trim(), 50);
            if s.is_empty() {
                unique_username(&state, local).await?
            } else {
                unique_username(&state, &s).await?
            }
        }
        _ => unique_username(&state, local).await?,
    };
    let hash = runway_core::password::hash(&body.password)?;
    let user: User = sqlx::query_as(
        "INSERT INTO \"user\" (email, username, password_hash, email_verified)
         VALUES ($1, $2, $3, false) RETURNING *",
    )
    .bind(&email)
    .bind(&username)
    .bind(&hash)
    .fetch_one(&state.db)
    .await?;
    let user = ensure_personal_team(&state, user).await?;
    let jar = jar.add(session_cookie(&state, user.id)?);
    Ok((
        axum::http::StatusCode::CREATED,
        (jar, axum::Json(json!({ "ok": true }))),
    )
        .into_response())
}

/// Create the personal team + owner membership on first login.
pub(crate) async fn ensure_personal_team(state: &AppState, user: User) -> ApiResult<User> {
    if user.default_team_id.is_some() {
        return Ok(user);
    }
    let team = Team::new(&format!("{}'s team", user.username), user.id);
    sqlx::query("INSERT INTO team (id, name, created_by_user_id) VALUES ($1, $2, $3)")
        .bind(&team.id)
        .bind(&team.name)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    runway_core::models::Project::assign_team_slug(&state.db, &team).await?;
    sqlx::query("INSERT INTO team_member (team_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(&team.id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE \"user\" SET default_team_id = $1 WHERE id = $2")
        .bind(&team.id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?,
    )
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> ApiResult<Response> {
    let jar = jar.remove(
        Cookie::build((state.settings.session_cookie.clone(), String::new()))
            .path("/")
            .build(),
    );
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

pub async fn me(user: AuthUser) -> ApiResult<Response> {
    Ok(axum::Json(json!({
        "id": user.user.id,
        "email": user.user.email,
        "username": user.user.username,
        "name": user.user.name,
        "default_team_id": user.user.default_team_id,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct UpdateMe {
    name: Option<String>,
    username: Option<String>,
    email: Option<String>,
}

/// `PATCH /api/auth/me` — profile fields. Username is slugified and must
/// be unique; a new email drops `email_verified` (devpush semantics).
pub async fn update_me(
    user: AuthUser,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateMe>,
) -> ApiResult<Response> {
    let uid = user.user.id;
    if let Some(name) = &body.name {
        let name = name.trim().chars().take(256).collect::<String>();
        sqlx::query("UPDATE \"user\" SET name = $1, updated_at = now() WHERE id = $2")
            .bind(&name)
            .bind(uid)
            .execute(&state.db)
            .await?;
    }
    if let Some(username) = &body.username {
        let username = username.trim();
        if !username.is_empty() {
            let slug = slugify(username, 50);
            if slug.is_empty() {
                return Err(ApiError::bad_request("invalid username"));
            }
            let taken: Option<(i64,)> =
                sqlx::query_as("SELECT id FROM \"user\" WHERE username = $1 AND id <> $2")
                    .bind(&slug)
                    .bind(uid)
                    .fetch_optional(&state.db)
                    .await?;
            if taken.is_some() {
                return Err(ApiError::bad_request("username already taken"));
            }
            sqlx::query("UPDATE \"user\" SET username = $1, updated_at = now() WHERE id = $2")
                .bind(&slug)
                .bind(uid)
                .execute(&state.db)
                .await?;
        }
    }
    if let Some(email) = &body.email {
        let email = email.trim().to_lowercase();
        if !email.is_empty() && email != user.user.email {
            if !email.contains('@') || email.len() > 320 {
                return Err(ApiError::bad_request("invalid email"));
            }
            let taken: Option<(i64,)> =
                sqlx::query_as("SELECT id FROM \"user\" WHERE email = $1 AND id <> $2")
                    .bind(&email)
                    .bind(uid)
                    .fetch_optional(&state.db)
                    .await?;
            if taken.is_some() {
                return Err(ApiError::bad_request("email already in use"));
            }
            sqlx::query(
                "UPDATE \"user\" SET email = $1, email_verified = false, updated_at = now() \
                 WHERE id = $2",
            )
            .bind(&email)
            .bind(uid)
            .execute(&state.db)
            .await?;
        }
    }
    let u: User = sqlx::query_as("SELECT * FROM \"user\" WHERE id = $1")
        .bind(uid)
        .fetch_one(&state.db)
        .await?;
    Ok(axum::Json(json!({
        "id": u.id,
        "email": u.email,
        "username": u.username,
        "name": u.name,
        "default_team_id": u.default_team_id,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct ChangePassword {
    current_password: String,
    new_password: String,
}

/// `POST /api/auth/password` — change the account password. OIDC-only
/// accounts (no hash) can't use this.
pub async fn change_password(
    user: AuthUser,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<ChangePassword>,
) -> ApiResult<Response> {
    let Some(hash) = user.user.password_hash.as_deref() else {
        return Err(ApiError::bad_request("account has no password"));
    };
    if !runway_core::password::verify(hash, &body.current_password) {
        return Err(ApiError::unauthorized("current password is wrong"));
    }
    if body.new_password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    let hash = runway_core::password::hash(&body.new_password)?;
    sqlx::query("UPDATE \"user\" SET password_hash = $1, updated_at = now() WHERE id = $2")
        .bind(&hash)
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct DeleteMe {
    password: String,
}

/// `DELETE /api/auth/me` — password-confirmed account deletion. Marks the
/// row `deleted` (devpush semantics; a cleanup job owns real removal).
pub async fn delete_me(
    user: AuthUser,
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Json(body): axum::Json<DeleteMe>,
) -> ApiResult<Response> {
    let ok = user
        .user
        .password_hash
        .as_deref()
        .map(|h| runway_core::password::verify(h, &body.password))
        .unwrap_or(false);
    if !ok {
        return Err(ApiError::unauthorized("password is wrong"));
    }
    sqlx::query("UPDATE \"user\" SET status = 'deleted', updated_at = now() WHERE id = $1")
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    let jar = jar.remove(
        Cookie::build((state.settings.session_cookie.clone(), String::new()))
            .path("/")
            .build(),
    );
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

pub(crate) async fn unique_username(state: &AppState, login: &str) -> ApiResult<String> {
    let base = {
        let s = slugify(login, 50);
        if s.is_empty() {
            format!("user-{}", token_hex(4))
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 0;
    loop {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM \"user\" WHERE username = $1")
            .bind(&candidate)
            .fetch_optional(&state.db)
            .await?;
        if exists.is_none() {
            return Ok(candidate);
        }
        n += 1;
        candidate = format!("{base}-{n}");
    }
}
