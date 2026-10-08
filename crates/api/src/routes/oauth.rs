//! Dedicated OAuth sign-in — `Continue with GitHub` / `Google`
//! (devpush `auth.py` github/google routes). Generic OIDC covers
//! enterprise IdPs; these cover the social providers that need no
//! IdP at all. The GitHub token also lands in `user_identity` where
//! the rest of the app can pick it up for personal-repo access.
//!
//! - `GET /api/auth/{provider}`          — redirect to the provider
//! - `GET /api/auth/{provider}/callback` — code exchange → session
//!
//! CSRF via `oauth_state_{provider}` cookie, same pattern as OIDC.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
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

struct Provider {
    authorize: &'static str,
    token: &'static str,
    scope: &'static str,
}

fn provider(name: &str) -> ApiResult<Provider> {
    match name {
        "github" => Ok(Provider {
            authorize: "https://github.com/login/oauth/authorize",
            token: "https://github.com/login/oauth/access_token",
            scope: "read:user user:email",
        }),
        "google" => Ok(Provider {
            authorize: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
            scope: "openid email profile",
        }),
        _ => Err(ApiError::bad_request("unknown provider")),
    }
}

fn creds<'a>(state: &'a AppState, name: &str) -> ApiResult<(&'a str, &'a str)> {
    let (id, secret) = match name {
        "github" => (
            &state.settings.github_oauth_client_id,
            &state.settings.github_oauth_client_secret,
        ),
        "google" => (
            &state.settings.google_oauth_client_id,
            &state.settings.google_oauth_client_secret,
        ),
        _ => return Err(ApiError::bad_request("unknown provider")),
    };
    Ok((
        id.as_deref()
            .ok_or_else(|| ApiError::bad_request(format!("{name} OAuth is not configured")))?,
        secret.as_deref().unwrap_or(""),
    ))
}

fn callback_uri(state: &AppState, name: &str) -> String {
    format!(
        "{}://{}/api/auth/oauth/{name}/callback",
        state.settings.url_scheme, state.settings.app_hostname
    )
}

fn state_cookie_name(name: &str) -> String {
    format!("oauth_state_{name}")
}

/// `GET /api/auth/{provider}` — kick off the authorize redirect.
pub async fn authorize(
    Path(name): Path<String>,
    State(state): State<AppState>,
    jar: CookieJar,
) -> ApiResult<Response> {
    let p = provider(&name)?;
    let (client_id, _) = creds(&state, &name)?;
    let redirect_uri = callback_uri(&state, &name);
    let state_token = token_hex(16);
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        p.authorize,
        urlencoding(client_id),
        urlencoding(&redirect_uri),
        urlencoding(p.scope),
        state_token,
    );
    let cookie = Cookie::build((state_cookie_name(&name), state_token))
        .path("/")
        .http_only(true)
        .max_age(time::Duration::minutes(10))
        .build();
    Ok((jar.add(cookie), Redirect::to(&url)).into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

struct ExternalUser {
    provider_user_id: String,
    email: String,
    name: Option<String>,
    metadata: Value,
    access_token: String,
}

/// Exchange `code` and fetch the provider profile.
async fn fetch_user(state: &AppState, name: &str, code: &str) -> ApiResult<ExternalUser> {
    let p = provider(name)?;
    let (client_id, client_secret) = creds(state, name)?;
    let redirect_uri = callback_uri(state, name);
    let http = reqwest::Client::new();

    let token_res: Value = http
        .post(p.token)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .map_err(|_| ApiError::unauthorized("oauth exchange failed"))?
        .json()
        .await
        .map_err(|_| ApiError::unauthorized("oauth exchange failed"))?;
    let access_token = token_res["access_token"]
        .as_str()
        .ok_or_else(|| ApiError::unauthorized("oauth exchange failed"))?
        .to_string();

    if name == "github" {
        let api = state.settings.github_api_url.trim_end_matches('/');
        let user: Value = http
            .get(format!("{api}/user"))
            .bearer_auth(&access_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await
            .map_err(|_| ApiError::unauthorized("github userinfo failed"))?
            .json()
            .await
            .map_err(|_| ApiError::unauthorized("github userinfo failed"))?;
        let id = user["id"]
            .as_i64()
            .ok_or_else(|| ApiError::bad_request("github returned no user id"))?;
        let mut email = user["email"].as_str().unwrap_or("").to_lowercase();
        if email.is_empty() {
            // Private email — pull the verified primary address.
            let emails: Value = http
                .get(format!("{api}/user/emails"))
                .bearer_auth(&access_token)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "runway")
                .send()
                .await
                .map_err(|_| ApiError::unauthorized("github emails failed"))?
                .json()
                .await
                .map_err(|_| ApiError::unauthorized("github emails failed"))?;
            email = emails
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .find(|e| {
                            e["primary"].as_bool() == Some(true)
                                && e["verified"].as_bool() == Some(true)
                        })
                        .or_else(|| a.first())
                })
                .and_then(|e| e["email"].as_str())
                .unwrap_or("")
                .to_lowercase();
        }
        let login = user["login"].as_str().unwrap_or("user").to_string();
        return Ok(ExternalUser {
            provider_user_id: id.to_string(),
            // devpush parity: a verified email wins; otherwise a stable
            // github.local placeholder keeps the account creatable.
            email: if email.is_empty() {
                format!("{login}@github.local")
            } else {
                email
            },
            name: user["name"].as_str().map(String::from),
            metadata: json!({
                "login": login,
                "name": user["name"].as_str(),
            }),
            access_token,
        });
    }

    // google — OIDC userinfo endpoint.
    let info: Value = http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|_| ApiError::unauthorized("google userinfo failed"))?
        .json()
        .await
        .map_err(|_| ApiError::unauthorized("google userinfo failed"))?;
    let sub = info["sub"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("google returned no user id"))?
        .to_string();
    let email = info["email"].as_str().unwrap_or("").to_lowercase();
    if email.is_empty() {
        return Err(ApiError::bad_request("google returned no email"));
    }
    Ok(ExternalUser {
        provider_user_id: sub,
        email,
        name: info["name"].as_str().map(String::from),
        metadata: json!({
            "email": info["email"].as_str(),
            "name": info["name"].as_str(),
            "picture": info["picture"].as_str(),
        }),
        access_token,
    })
}

/// `GET /api/auth/{provider}/callback` — exchange, resolve/create the
/// user (allowlist-gated), upsert the identity, mint a session.
pub async fn callback(
    Path(name): Path<String>,
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<CallbackQuery>,
) -> ApiResult<Response> {
    provider(&name)?;
    let cookie_name = state_cookie_name(&name);
    let expected = jar.get(&cookie_name).map(|c| c.value().to_string());
    if expected.as_deref() != Some(params.state.as_str()) {
        return Err(ApiError::unauthorized("invalid oauth state"));
    }
    let ext = fetch_user(&state, &name, &params.code).await?;

    let existing: Option<UserIdentity> =
        sqlx::query_as("SELECT * FROM user_identity WHERE provider = $1 AND provider_user_id = $2")
            .bind(&name)
            .bind(&ext.provider_user_id)
            .fetch_optional(&state.db)
            .await?;

    let session_user = jar
        .get(&state.settings.session_cookie)
        .and_then(|c| crate::auth::session_user_id(&state.settings.secret_key, c.value()));

    let user: User = if let Some(identity) = existing {
        if session_user.is_some_and(|uid| uid != identity.user_id) {
            return Err(ApiError::conflict(
                "this account is already linked to another user",
            ));
        }
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(identity.user_id)
            .fetch_one(&state.db)
            .await?
    } else if let Some(uid) = session_user {
        // Link mode — an active session means "connect to me".
        sqlx::query_as::<_, User>("SELECT * FROM \"user\" WHERE id = $1")
            .bind(uid)
            .fetch_one(&state.db)
            .await?
    } else {
        let user: Option<User> = sqlx::query_as("SELECT * FROM \"user\" WHERE email = $1")
            .bind(&ext.email)
            .fetch_optional(&state.db)
            .await?;
        match user {
            Some(u) => u,
            None => {
                // github.local placeholders aren't real mailboxes — let
                // them through the allowlist only when it's empty.
                let real_email = !ext.email.ends_with("@github.local");
                if !is_email_allowed(&state.db, &ext.email).await? {
                    if real_email {
                        notify_denied(&state, &ext.email, &name).await;
                    }
                    return Err(ApiError::forbidden(
                        state.settings.access_denied_message.clone(),
                    ));
                }
                let local = ext.email.split('@').next().unwrap_or("user");
                let username = unique_username(&state, local).await?;
                let u = sqlx::query_as::<_, User>(
                    "INSERT INTO \"user\" (email, username, name, email_verified)
                     VALUES ($1, $2, $3, $4) RETURNING *",
                )
                .bind(&ext.email)
                .bind(&username)
                .bind(&ext.name)
                .bind(real_email)
                .fetch_one(&state.db)
                .await?;
                ensure_personal_team(&state, u).await?
            }
        }
    };

    // Upsert the identity row — uniqueness is (provider, provider_user_id).
    let enc_token = state.crypto.encrypt(&ext.access_token)?;
    sqlx::query(
        "INSERT INTO user_identity
         (user_id, provider, provider_user_id, access_token, provider_metadata)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (provider, provider_user_id)
         DO UPDATE SET access_token = EXCLUDED.access_token,
                       provider_metadata = EXCLUDED.provider_metadata,
                       updated_at = now()",
    )
    .bind(user.id)
    .bind(&name)
    .bind(&ext.provider_user_id)
    .bind(&enc_token)
    .bind(&ext.metadata)
    .execute(&state.db)
    .await?;

    let jar = jar.add(session_cookie(&state, user.id)?);
    let jar = jar.remove(
        Cookie::build((state_cookie_name(&name), String::new()))
            .path("/")
            .build(),
    );
    Ok((jar, Redirect::to("/")).into_response())
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
