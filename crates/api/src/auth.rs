//! Auth: session JWT (HS256 cookie) + `ak_` API keys (Bearer) + `dp_` deploy tokens.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use runway_core::crypto::sha256_hex;
use runway_core::models::{ApiKey, User};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sub: i64,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
}

pub fn mint_session(secret: &str, user_id: i64, max_age: u64) -> anyhow::Result<String> {
    let now = Utc::now().timestamp() as u64;
    let claims = SessionClaims {
        sub: user_id,
        iss: "runway".into(),
        aud: "runway:auth".into(),
        iat: now,
        exp: now + max_age,
    };
    Ok(jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

fn decode_session(secret: &str, token: &str) -> Option<SessionClaims> {
    let mut validation = Validation::default();
    validation.set_audience(&["runway:auth"]);
    jsonwebtoken::decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

/// User id from a session JWT, if valid and unexpired.
pub(crate) fn session_user_id(secret: &str, token: &str) -> Option<i64> {
    decode_session(secret, token)
        .filter(|c| c.exp > Utc::now().timestamp() as u64)
        .map(|c| c.sub)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginClaims {
    /// Email being signed in.
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
}

/// Short-lived single-purpose token for magic-link login.
/// Stateless — single-use is not enforced; expiry (15 min) bounds the window.
pub fn mint_login_token(secret: &str, email: &str) -> anyhow::Result<String> {
    let now = Utc::now().timestamp() as u64;
    let claims = LoginClaims {
        sub: email.to_string(),
        iss: "runway".into(),
        aud: "runway:login".into(),
        iat: now,
        exp: now + 15 * 60,
    };
    Ok(jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?)
}

pub fn decode_login_token(secret: &str, token: &str) -> Option<String> {
    let mut validation = Validation::default();
    validation.set_audience(&["runway:login"]);
    jsonwebtoken::decode::<LoginClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims.sub)
}

/// Authenticated user — cookie session or `Authorization: Bearer ak_...`.
pub struct AuthUser {
    pub user: User,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // Bearer api key first.
        if let Some(token) = bearer_token(parts) {
            if let Some(raw) = token.strip_prefix("ak_") {
                let hash = sha256_hex(&format!("ak_{raw}"));
                let key: Option<ApiKey> =
                    sqlx::query_as("SELECT * FROM api_key WHERE token = $1 AND status = 'active'")
                        .bind(&hash)
                        .fetch_optional(&state.db)
                        .await
                        .map_err(ApiError::from)?;
                if let Some(key) = key {
                    sqlx::query("UPDATE api_key SET last_used_at = now() WHERE id = $1")
                        .bind(&key.id)
                        .execute(&state.db)
                        .await
                        .ok();
                    let user = user_by_id(state, key.user_id).await?;
                    return Ok(Self { user });
                }
            }
            return Err(ApiError::unauthorized("invalid credentials"));
        }

        // Cookie session.
        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = cookie_header
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(k, _)| *k == state.settings.session_cookie)
            .map(|(_, v)| v.to_string());
        let Some(token) = token else {
            return Err(ApiError::unauthorized("authentication required"));
        };
        let Some(claims) = decode_session(&state.settings.secret_key, &token) else {
            return Err(ApiError::unauthorized("invalid session"));
        };
        let user = user_by_id(state, claims.sub).await?;
        if let Some(cutoff) = user.tokens_invalid_before {
            if (claims.iat as i64) < cutoff.timestamp() {
                return Err(ApiError::unauthorized("session revoked"));
            }
        }
        Ok(Self { user })
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(String::from)
}

pub async fn user_by_id(state: &AppState, id: i64) -> Result<User, ApiError> {
    let user: Option<User> =
        sqlx::query_as("SELECT * FROM \"user\" WHERE id = $1 AND status = 'active'")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(ApiError::from)?;
    user.ok_or_else(|| ApiError::unauthorized("user not found"))
}

/// Verify a `dp_` deploy token; returns (token row, project id).
pub async fn verify_deploy_token(
    state: &AppState,
    raw: &str,
) -> Result<runway_core::models::DeployToken, ApiError> {
    let hash = sha256_hex(raw);
    let token: Option<runway_core::models::DeployToken> =
        sqlx::query_as("SELECT * FROM deploy_token WHERE token = $1 AND status = 'active'")
            .bind(&hash)
            .fetch_optional(&state.db)
            .await
            .map_err(ApiError::from)?;
    let Some(token) = token else {
        return Err(ApiError::unauthorized("invalid deploy token"));
    };
    sqlx::query("UPDATE deploy_token SET last_used_at = now() WHERE id = $1")
        .bind(&token.id)
        .execute(&state.db)
        .await
        .ok();
    Ok(token)
}
