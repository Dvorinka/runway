//! GitHub OAuth login: `/api/auth/github` redirects, callback upserts the
//! user + default team + membership, sets the session cookie.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::json;

use runway_core::models::{Team, User, UserIdentity};
use runway_core::slugify::{slugify, token_hex};

use crate::auth::{mint_session, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub async fn github_login(State(state): State<AppState>, jar: CookieJar) -> ApiResult<Response> {
    let Some(gh) = &state.github_oauth else {
        return Err(ApiError::bad_request("GitHub OAuth is not configured"));
    };
    let state_token = token_hex(16);
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}://{}/api/auth/github/callback&scope=read:user,user:email&state={}",
        gh.client_id(),
        state.settings.url_scheme,
        state.settings.app_hostname,
        state_token,
    );
    let cookie = Cookie::build(("oauth_state", state_token))
        .path("/")
        .http_only(true)
        .max_age(time::Duration::minutes(10))
        .build();
    Ok((jar.add(cookie), Redirect::to(&url)).into_response())
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

pub async fn github_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<CallbackParams>,
) -> ApiResult<Response> {
    let Some(gh) = &state.github_oauth else {
        return Err(ApiError::bad_request("GitHub OAuth is not configured"));
    };
    let expected = jar.get("oauth_state").map(|c| c.value().to_string());
    if expected.as_deref() != Some(params.state.as_str()) {
        return Err(ApiError::unauthorized("invalid oauth state"));
    }

    let token = gh
        .exchange_code(&params.code)
        .await
        .map_err(|_| ApiError::unauthorized("oauth exchange failed"))?;
    let (github_id, login, name) = gh.user_info(&token).await.map_err(ApiError::internal)?;
    let email = gh
        .user_primary_email(&token)
        .await
        .map_err(ApiError::internal)?
        .unwrap_or_else(|| format!("{login}@users.noreply.github.com"));

    // Upsert user by identity.
    let existing: Option<UserIdentity> = sqlx::query_as(
        "SELECT * FROM user_identity WHERE provider = 'github' AND provider_user_id = $1",
    )
    .bind(github_id.to_string())
    .fetch_optional(&state.db)
    .await?;

    let user: User = if let Some(identity) = existing {
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(identity.user_id)
            .fetch_one(&state.db)
            .await?
    } else {
        // Reuse account when the email already exists.
        let user: Option<User> = sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
        let user = match user {
            Some(u) => u,
            None => {
                let username = unique_username(&state, &login).await?;
                sqlx::query_as::<_, User>(
                    "INSERT INTO \"user\" (email, username, name, email_verified)
                     VALUES ($1, $2, $3, true) RETURNING *",
                )
                .bind(&email)
                .bind(&username)
                .bind(&name)
                .fetch_one(&state.db)
                .await?
            }
        };

        let enc_token = state.crypto.encrypt(&token)?;
        sqlx::query(
            "INSERT INTO user_identity
             (user_id, provider, provider_user_id, access_token, provider_metadata)
             VALUES ($1, 'github', $2, $3, $4)",
        )
        .bind(user.id)
        .bind(github_id.to_string())
        .bind(&enc_token)
        .bind(json!({ "login": login }))
        .execute(&state.db)
        .await?;

        ensure_personal_team(&state, user).await?
    };

    let jar = jar.add(session_cookie(&state, user.id)?);
    let jar = jar.remove(Cookie::from("oauth_state"));
    Ok((jar, Redirect::to("/")).into_response())
}

fn session_cookie(state: &AppState, user_id: i64) -> ApiResult<Cookie<'static>> {
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

// ---------------------------------------------------------------------------
// Magic link
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct MagicLinkBody {
    email: String,
}

/// `POST /api/auth/magic-link` — email a sign-in link.
/// No SMTP configured → the link is logged (dev mode) and a generic 200
/// returned either way to avoid account enumeration.
pub async fn magic_link(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<MagicLinkBody>,
) -> ApiResult<Response> {
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(ApiError::bad_request("invalid email"));
    }
    let token = crate::auth::mint_login_token(&state.settings.secret_key, &email)
        .map_err(ApiError::internal)?;
    let link = format!(
        "{}://{}/api/auth/magic-link/verify?token={token}",
        state.settings.url_scheme, state.settings.app_hostname,
    );
    runway_core::mail::send(
        &state.settings,
        &email,
        "Sign in to Runway",
        &format!("Sign in to Runway:\n\n{link}\n\nThis link expires in 15 minutes."),
    )
    .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct MagicLinkVerify {
    token: String,
}

/// `GET /api/auth/magic-link/verify` — upsert user by email, set session.
pub async fn magic_link_verify(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<MagicLinkVerify>,
) -> ApiResult<Response> {
    let Some(email) = crate::auth::decode_login_token(&state.settings.secret_key, &params.token)
    else {
        return Err(ApiError::unauthorized("invalid or expired link"));
    };

    let user: Option<User> = sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;
    let user = match user {
        Some(u) => u,
        None => {
            let local = email.split('@').next().unwrap_or("user");
            let username = unique_username(&state, local).await?;
            sqlx::query_as::<_, User>(
                "INSERT INTO \"user\" (email, username, email_verified)
                 VALUES ($1, $2, true) RETURNING *",
            )
            .bind(&email)
            .bind(&username)
            .fetch_one(&state.db)
            .await?
        }
    };
    let user = ensure_personal_team(&state, user).await?;

    let jar = jar.add(session_cookie(&state, user.id)?);
    Ok((jar, Redirect::to("/")).into_response())
}

/// Create the personal team + owner membership on first login.
async fn ensure_personal_team(state: &AppState, user: User) -> ApiResult<User> {
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
    let jar = jar.remove(Cookie::from(state.settings.session_cookie.clone()));
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

async fn unique_username(state: &AppState, login: &str) -> ApiResult<String> {
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
