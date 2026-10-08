//! OIDC / SSO login + account linking — superset of devpush
//! `routers/oidc.py` (which only links to an existing session). We
//! also log in when no session exists: SSO is useless as link-only.
//!
//! - `GET /api/auth/oidc`          — redirect to the provider
//! - `GET /api/auth/oidc/callback` — code exchange → userinfo → session
//!
//! Discovery doc is fetched per flow (cold path — no cache).
//! CSRF via `oidc_state` cookie, same pattern as the GitHub flow.

use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::access::is_email_allowed;
use runway_core::models::{User, UserIdentity};
use runway_core::slugify::token_hex;

use crate::error::{ApiError, ApiResult};
use crate::routes::auth::{ensure_personal_team, notify_denied, session_cookie, unique_username};
use crate::state::AppState;

/// Discovery doc subset.
struct Endpoints {
    authorize: String,
    token: String,
    userinfo: String,
}

async fn discover(state: &AppState) -> ApiResult<Endpoints> {
    let url = state
        .settings
        .oidc_discovery_url
        .clone()
        .ok_or_else(|| ApiError::bad_request("OIDC is not configured"))?;
    let doc: Value = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("OIDC discovery failed: {e}")))?
        .json()
        .await
        .map_err(|e| ApiError::bad_request(format!("OIDC discovery failed: {e}")))?;
    let pick = |k: &str| -> ApiResult<String> {
        doc[k]
            .as_str()
            .map(String::from)
            .ok_or_else(|| ApiError::bad_request(format!("OIDC discovery missing {k}")))
    };
    Ok(Endpoints {
        authorize: pick("authorization_endpoint")?,
        token: pick("token_endpoint")?,
        userinfo: pick("userinfo_endpoint")?,
    })
}

/// `GET /api/auth/oidc` — kick off the authorize redirect.
pub async fn authorize(State(state): State<AppState>, jar: CookieJar) -> ApiResult<Response> {
    if !state.settings.oidc_configured() {
        return Err(ApiError::bad_request("OIDC is not configured"));
    }
    let ep = discover(&state).await?;
    let redirect_uri = format!(
        "{}://{}/api/auth/oidc/callback",
        state.settings.url_scheme, state.settings.app_hostname
    );
    let state_token = token_hex(16);
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&state={}",
        ep.authorize,
        state.settings.oidc_client_id.as_deref().unwrap_or(""),
        urlencoding(&redirect_uri),
        state_token,
    );
    let cookie = Cookie::build(("oidc_state", state_token))
        .path("/")
        .http_only(true)
        .max_age(time::Duration::minutes(10))
        .build();
    Ok((jar.add(cookie), Redirect::to(&url)).into_response())
}

#[derive(Deserialize)]
pub struct OidcCallback {
    code: String,
    state: String,
}

/// `GET /api/auth/oidc/callback` — exchange, userinfo, login or link.
pub async fn callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<OidcCallback>,
) -> ApiResult<Response> {
    if !state.settings.oidc_configured() {
        return Err(ApiError::bad_request("OIDC is not configured"));
    }
    let expected = jar.get("oidc_state").map(|c| c.value().to_string());
    if expected.as_deref() != Some(params.state.as_str()) {
        return Err(ApiError::unauthorized("invalid oidc state"));
    }
    let ep = discover(&state).await?;
    let redirect_uri = format!(
        "{}://{}/api/auth/oidc/callback",
        state.settings.url_scheme, state.settings.app_hostname
    );

    // Code → tokens.
    let http = reqwest::Client::new();
    let token_res: Value = http
        .post(&ep.token)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", params.code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            (
                "client_id",
                state.settings.oidc_client_id.as_deref().unwrap_or(""),
            ),
            (
                "client_secret",
                state.settings.oidc_client_secret.as_deref().unwrap_or(""),
            ),
        ])
        .send()
        .await
        .map_err(|_| ApiError::unauthorized("oidc exchange failed"))?
        .json()
        .await
        .map_err(|_| ApiError::unauthorized("oidc exchange failed"))?;
    let access_token = token_res["access_token"]
        .as_str()
        .ok_or_else(|| ApiError::unauthorized("oidc exchange failed"))?
        .to_string();

    // Tokens → userinfo.
    let info: Value = http
        .get(&ep.userinfo)
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|_| ApiError::unauthorized("oidc userinfo failed"))?
        .json()
        .await
        .map_err(|_| ApiError::unauthorized("oidc userinfo failed"))?;
    let sub = info["sub"]
        .as_str()
        .or_else(|| info["id"].as_str())
        .ok_or_else(|| ApiError::bad_request("OIDC provider returned no user id"))?
        .to_string();
    let email = info["email"].as_str().unwrap_or("").to_lowercase();
    let name = info["name"].as_str().map(String::from);

    // Existing identity → straight login.
    let existing: Option<UserIdentity> = sqlx::query_as(
        "SELECT * FROM user_identity WHERE provider = 'oidc' AND provider_user_id = $1",
    )
    .bind(&sub)
    .fetch_optional(&state.db)
    .await?;

    let session_user = jar
        .get(&state.settings.session_cookie)
        .and_then(|c| crate::auth::session_user_id(&state.settings.secret_key, c.value()));

    let user: User = if let Some(identity) = existing {
        if session_user.is_some_and(|uid| uid != identity.user_id) {
            // Sessioned as A, identity belongs to B — devpush errors.
            return Err(ApiError::conflict(
                "this OIDC account is already linked to another user",
            ));
        }
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(identity.user_id)
            .fetch_one(&state.db)
            .await?
    } else if let Some(uid) = session_user {
        // Link mode: an active session means "connect this OIDC account
        // to me" (devpush parity).
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(uid)
            .fetch_one(&state.db)
            .await?
    } else {
        // Login/signup — allowlist gate on new accounts.
        if email.is_empty() {
            return Err(ApiError::bad_request("OIDC provider returned no email"));
        }
        let user: Option<User> = sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
        match user {
            Some(u) => u,
            None => {
                if !is_email_allowed(&state.db, &email).await? {
                    notify_denied(&state, &email, "oidc").await;
                    return Err(ApiError::forbidden(
                        state.settings.access_denied_message.clone(),
                    ));
                }
                let local = email.split('@').next().unwrap_or("user");
                let username = unique_username(&state, local).await?;
                let u = sqlx::query_as::<_, User>(
                    "INSERT INTO \"user\" (email, username, name, email_verified)
                     VALUES ($1, $2, $3, true) RETURNING *",
                )
                .bind(&email)
                .bind(&username)
                .bind(&name)
                .fetch_one(&state.db)
                .await?;
                ensure_personal_team(&state, u).await?
            }
        }
    };

    // Upsert the identity row — uniqueness is (provider, provider_user_id).
    let enc_token = state.crypto.encrypt(&access_token)?;
    sqlx::query(
        "INSERT INTO user_identity
         (user_id, provider, provider_user_id, access_token, provider_metadata)
         VALUES ($1, 'oidc', $2, $3, $4)
         ON CONFLICT (provider, provider_user_id)
         DO UPDATE SET access_token = EXCLUDED.access_token,
                       provider_metadata = EXCLUDED.provider_metadata",
    )
    .bind(user.id)
    .bind(&sub)
    .bind(&enc_token)
    .bind(json!({ "email": email, "name": name }))
    .execute(&state.db)
    .await?;

    let jar = jar.add(session_cookie(&state, user.id)?);
    let jar = jar.remove(
        Cookie::build(("oidc_state", String::new()))
            .path("/")
            .build(),
    );
    Ok((jar, Redirect::to("/")).into_response())
}

/// `GET /api/auth/oidc/info` — frontend login-page metadata (button
/// label + whether SSO is enabled). Unauthenticated by design.
pub async fn info(State(state): State<AppState>) -> ApiResult<Response> {
    Ok(Json(json!({
        "enabled": state.settings.oidc_configured(),
        "display_name": state.settings.oidc_display_name,
    }))
    .into_response())
}

/// Minimal percent-encoding for the redirect_uri (avoid a urlencoding dep).
fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
