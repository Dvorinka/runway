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

use crate::auth::{mint_pending, mint_session, AuthUser};
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
    let user = user.unwrap();
    if user.totp_enabled {
        // Password verified — second factor required. The pending
        // token is a short-lived JWT with a distinct audience; it can
        // never pass as a session (decode_session pins runway:auth).
        let pending =
            mint_pending(&state.settings.secret_key, user.id).map_err(ApiError::internal)?;
        return Ok(axum::Json(json!({
            "two_factor": true,
            "pending": pending,
        }))
        .into_response());
    }
    let user = ensure_personal_team(&state, user).await?;
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
    if body.password.is_empty() {
        return Err(ApiError::bad_request("password must not be empty"));
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
        "has_avatar": user.user.has_avatar,
        "totp_enabled": user.user.totp_enabled,
        "email_verified": user.user.email_verified,
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
    jar: CookieJar,
    axum::Json(body): axum::Json<ChangePassword>,
) -> ApiResult<Response> {
    let Some(hash) = user.user.password_hash.as_deref() else {
        return Err(ApiError::bad_request("account has no password"));
    };
    if !runway_core::password::verify(hash, &body.current_password) {
        return Err(ApiError::unauthorized("current password is wrong"));
    }
    if body.new_password.is_empty() {
        return Err(ApiError::bad_request("password must not be empty"));
    }
    let hash = runway_core::password::hash(&body.new_password)?;
    // Revoke every pre-existing session (JWTs are checked against this
    // cutoff), then hand the caller a fresh cookie so they stay signed in.
    sqlx::query(
        "UPDATE \"user\" SET password_hash = $1, tokens_invalid_before = now(),
         updated_at = now() WHERE id = $2",
    )
    .bind(&hash)
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    let jar = jar.add(session_cookie(&state, user.user.id)?);
    Ok((jar, axum::Json(json!({ "ok": true }))).into_response())
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
    sqlx::query(
        "UPDATE \"user\" SET status = 'deleted', tokens_invalid_before = now(),
         updated_at = now() WHERE id = $1",
    )
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    // Cascade — sole-owner teams die with the account. Port of devpush
    // enqueueing delete_user.
    runway_core::deploy::enqueue(
        &state.db,
        "delete_user",
        serde_json::json!({ "user_id": user.user.id }),
        0,
    )
    .await
    .map_err(ApiError::internal)?;
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

// ---- TOTP two-factor ------------------------------------------------

fn totp_for(secret_b32: &str, email: &str) -> ApiResult<totp_rs::TOTP> {
    use totp_rs::{Algorithm, Secret};
    let secret = Secret::Encoded(secret_b32.to_string());
    totp_rs::TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret
            .to_bytes()
            .map_err(|_| ApiError::bad_request("invalid totp secret"))?,
        Some("Runway".into()),
        email.to_string(),
    )
    .map_err(ApiError::internal)
}

fn check_totp(user: &User, code: &str, secret_b32: &str) -> bool {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    code.len() == 6
        && totp_for(secret_b32, &user.email)
            .map(|t| t.check_current(&code).unwrap_or(false))
            .unwrap_or(false)
}

fn recovery_codes() -> Vec<String> {
    (0..8)
        .map(|_| format!("{}-{}", token_hex(2), token_hex(2)))
        .collect()
}

/// `POST /api/auth/totp/enroll` — generate a pending secret. The raw
/// base32 goes back once for the authenticator app; the encrypted copy
/// stays pending until `verify` confirms a code.
pub async fn totp_enroll(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    use totp_rs::Secret;
    if user.user.totp_enabled {
        return Err(ApiError::bad_request("two-factor already enabled"));
    }
    let secret = Secret::generate_secret().to_encoded().to_string();
    let enc = state.crypto.encrypt(&secret).map_err(ApiError::internal)?;
    sqlx::query("UPDATE \"user\" SET totp_secret_enc = $1, updated_at = now() WHERE id = $2")
        .bind(&enc)
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    let totp = totp_for(&secret, &user.user.email)?;
    Ok(axum::Json(json!({
        "secret": secret,
        "otpauth_url": totp.get_url(),
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct TotpCode {
    code: String,
}

/// `POST /api/auth/totp/verify` — first valid code activates 2FA and
/// returns one-time recovery codes (stored as sha256 hashes).
pub async fn totp_verify(
    user: AuthUser,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<TotpCode>,
) -> ApiResult<Response> {
    let enc = user
        .user
        .totp_secret_enc
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("enroll first"))?;
    let secret = state.crypto.decrypt(enc).map_err(ApiError::internal)?;
    if !check_totp(&user.user, &body.code, &secret) {
        return Err(ApiError::unauthorized("invalid code"));
    }
    let codes = recovery_codes();
    let hashes: Vec<String> = codes
        .iter()
        .map(|c| runway_core::crypto::sha256_hex(c))
        .collect();
    sqlx::query(
        "UPDATE \"user\" SET totp_enabled = true, totp_recovery = $1,
                updated_at = now() WHERE id = $2",
    )
    .bind(json!(hashes))
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    Ok(axum::Json(json!({ "ok": true, "recovery_codes": codes })).into_response())
}

#[derive(Deserialize)]
pub struct TotpChallenge {
    pending: String,
    code: String,
}

/// `POST /api/auth/totp/challenge` — pending token + TOTP or recovery
/// code → session cookie. Recovery codes are single-use and consumed.
pub async fn totp_challenge(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::Json(body): axum::Json<TotpChallenge>,
) -> ApiResult<Response> {
    let Some(uid) = crate::auth::decode_pending(&state.settings.secret_key, &body.pending) else {
        return Err(ApiError::unauthorized("invalid or expired challenge"));
    };
    let user = crate::auth::user_by_id(&state, uid).await?;
    if !user.totp_enabled {
        return Err(ApiError::bad_request("two-factor not enabled"));
    }
    let enc = user
        .totp_secret_enc
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("enroll first"))?;
    let secret = state.crypto.decrypt(enc).map_err(ApiError::internal)?;

    let code: String = body.code.trim().to_lowercase();
    let mut ok = check_totp(&user, &code, &secret);
    if !ok {
        // Recovery path — compare against stored hashes, consume on hit.
        let hashes: Vec<String> =
            serde_json::from_value(user.totp_recovery.clone()).unwrap_or_default();
        let hash = runway_core::crypto::sha256_hex(&code);
        if let Some(pos) = hashes.iter().position(|h| h == &hash) {
            let mut rest = hashes;
            rest.remove(pos);
            sqlx::query("UPDATE \"user\" SET totp_recovery = $1 WHERE id = $2")
                .bind(json!(rest))
                .bind(user.id)
                .execute(&state.db)
                .await?;
            ok = true;
        }
    }
    if !ok {
        return Err(ApiError::unauthorized("invalid code"));
    }
    let user = ensure_personal_team(&state, user).await?;
    let jar = jar.add(session_cookie(&state, user.id)?);
    Ok((jar, axum::Json(json!({ "ok": true }))).into_response())
}

#[derive(Deserialize)]
pub struct TotpDisable {
    password: String,
}

/// `POST /api/auth/totp/disable` — password re-auth required, then all
/// TOTP state is cleared.
pub async fn totp_disable(
    user: AuthUser,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<TotpDisable>,
) -> ApiResult<Response> {
    let valid = user
        .user
        .password_hash
        .as_deref()
        .map(|h| runway_core::password::verify(h, &body.password))
        .unwrap_or(false);
    if !valid {
        return Err(ApiError::unauthorized("invalid password"));
    }
    sqlx::query(
        "UPDATE \"user\" SET totp_enabled = false, totp_secret_enc = NULL,
                totp_recovery = '[]'::jsonb, updated_at = now() WHERE id = $1",
    )
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

// ---- Email verification + magic-link login -----------------------------

/// Insert a single-use `et_` token; returns the raw value for the mail link.
async fn mint_email_token(
    state: &AppState,
    user_id: Option<i64>,
    email: &str,
    kind: &str,
    ttl_secs: f64,
) -> ApiResult<String> {
    let raw = format!("et_{}", token_hex(24));
    let hash = runway_core::crypto::sha256_hex(&raw);
    sqlx::query(
        "INSERT INTO email_token (id, user_id, email, token, kind, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
    )
    .bind(token_hex(16))
    .bind(user_id)
    .bind(email)
    .bind(&hash)
    .bind(kind)
    .bind(ttl_secs)
    .execute(&state.db)
    .await?;
    Ok(raw)
}

fn verify_url(state: &AppState, raw: &str) -> String {
    format!(
        "{}://{}/api/auth/email/verify?token={raw}",
        state.settings.url_scheme, state.settings.app_hostname
    )
}

async fn send_token_mail(state: &AppState, email: &str, kind: &str, raw: &str) -> ApiResult<()> {
    let (subject, body) = if kind == "verify" {
        (
            "Verify your Runway email",
            format!(
                "Confirm this address for your Runway account:\n\n{}\n\n\
                 If you didn't request this, ignore the message.",
                verify_url(state, raw)
            ),
        )
    } else {
        (
            "Your Runway sign-in link",
            format!(
                "Sign in to Runway:\n\n{}\n\nThe link expires in 15 minutes and \
                 works once. If you didn't request it, ignore this message.",
                verify_url(state, raw)
            ),
        )
    };
    runway_core::mail::send(&state.settings, email, subject, &body)
        .await
        .map_err(ApiError::internal)
}

/// `GET /api/auth/providers` — public: which sign-in methods are on.
pub async fn providers(State(state): State<AppState>) -> Response {
    axum::Json(json!({
        "magic_link": runway_core::mail::smtp_configured(&state.settings),
        "github": state.settings.oauth_configured("github"),
        "google": state.settings.oauth_configured("google"),
    }))
    .into_response()
}

/// `POST /api/auth/email/resend` — send a verification link for the
/// signed-in user's address. No-op (204-ish ok) when already verified.
pub async fn email_resend(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    if !runway_core::mail::smtp_configured(&state.settings) {
        return Err(ApiError::bad_request("outgoing mail is not configured"));
    }
    if user.user.email_verified {
        return Ok(axum::Json(json!({ "ok": true, "verified": true })).into_response());
    }
    let raw = mint_email_token(
        &state,
        Some(user.user.id),
        &user.user.email,
        "verify",
        86400.0,
    )
    .await?;
    send_token_mail(&state, &user.user.email, "verify", &raw).await?;
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct MagicBody {
    email: String,
}

/// `POST /api/auth/email/login` — magic-link sign-in. Always returns ok so
/// the response can't enumerate accounts; the mail is only sent when the
/// address maps to an active user.
pub async fn email_login(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<MagicBody>,
) -> ApiResult<Response> {
    let email = body.email.trim().to_lowercase();
    if runway_core::mail::smtp_configured(&state.settings) && email.contains('@') {
        let user: Option<User> =
            sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1 AND status = 'active'")
                .bind(&email)
                .fetch_optional(&state.db)
                .await?;
        if let Some(user) = user {
            let raw = mint_email_token(&state, Some(user.id), &email, "login", 900.0).await?;
            if let Err(e) = send_token_mail(&state, &email, "login", &raw).await {
                tracing::warn!(error = %e.1, "magic-link mail failed");
            }
        }
    }
    Ok(axum::Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct EmailVerifyQuery {
    token: String,
}

/// `GET /api/auth/email/verify?token=…` — single-use consume.
/// `verify` marks the user's email confirmed; `login` mints a session.
/// Failures redirect to /login so a mistyped link lands somewhere useful.
pub async fn email_verify(
    State(state): State<AppState>,
    jar: CookieJar,
    axum::extract::Query(q): axum::extract::Query<EmailVerifyQuery>,
) -> ApiResult<Response> {
    let bad = axum::response::Redirect::to("/login?error=invalid_link").into_response();
    let hash = runway_core::crypto::sha256_hex(&q.token);
    // Atomic single-use consume — a replay finds used_at already set.
    let row: Option<(Option<i64>, String, String)> = sqlx::query_as(
        "UPDATE email_token SET used_at = now()
          WHERE token = $1 AND used_at IS NULL AND expires_at > now()
          RETURNING user_id, email, kind",
    )
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;
    let Some((user_id, email, kind)) = row else {
        return Ok(bad);
    };
    let Some(user_id) = user_id else {
        return Ok(bad);
    };
    match kind.as_str() {
        "verify" => {
            sqlx::query(
                "UPDATE \"user\" SET email_verified = true, updated_at = now()
                  WHERE id = $1 AND email = $2",
            )
            .bind(user_id)
            .bind(&email)
            .execute(&state.db)
            .await?;
            Ok(axum::response::Redirect::to("/settings?verified=1").into_response())
        }
        "login" => {
            let user = crate::auth::user_by_id(&state, user_id).await?;
            let user = ensure_personal_team(&state, user).await?;
            let jar = jar.add(session_cookie(&state, user.id)?);
            Ok((jar, axum::response::Redirect::to("/")).into_response())
        }
        _ => Ok(bad),
    }
}
