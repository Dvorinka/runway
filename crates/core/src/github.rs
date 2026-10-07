//! GitHub client: App JWT (RS256), installation tokens, repo lookups,
//! OAuth login exchange, webhook signature verification.
//! Port of devpush services/github.py.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::config::Settings;
use crate::crypto::Crypto;
use crate::error::{Error, Result};
use crate::models::GithubInstallation;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct GithubService {
    http: reqwest::Client,
    pub api_base: String,
    client_id: String,
    client_secret: String,
    app_id: String,
    private_key: String,
    jwt_cache: Arc<Mutex<Option<(String, Instant)>>>,
}

impl GithubService {
    /// App-level service (JWT + installation tokens + webhooks).
    pub fn from_settings(settings: &Settings) -> Option<Self> {
        if !settings.github_app_configured() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::new(),
            api_base: "https://api.github.com".into(),
            client_id: settings.github_client_id.clone().unwrap_or_default(),
            client_secret: settings.github_client_secret.clone().unwrap_or_default(),
            app_id: settings.github_app_id.clone().unwrap_or_default(),
            private_key: settings.github_app_private_key.clone().unwrap_or_default(),
            jwt_cache: Arc::new(Mutex::new(None)),
        })
    }

    /// OAuth-only service (login flow) — usable without App credentials.
    pub fn oauth_only(settings: &Settings) -> Option<Self> {
        if !settings.github_oauth_configured() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::new(),
            api_base: "https://api.github.com".into(),
            client_id: settings.github_client_id.clone()?,
            client_secret: settings.github_client_secret.clone()?,
            app_id: String::new(),
            private_key: String::new(),
            jwt_cache: Arc::new(Mutex::new(None)),
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// App JWT, cached for ~10 minutes (port of jwt_token property).
    pub fn jwt(&self) -> Result<String> {
        let mut cache = self.jwt_cache.lock().unwrap();
        if let Some((token, created)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(9 * 60) {
                return Ok(token.clone());
            }
        }
        let now = jsonwebtoken::get_current_timestamp();
        let claims = serde_json::json!({
            "iat": now as i64 - 60, // clock-skew buffer
            "exp": now + 10 * 60,
            "iss": self.app_id,
        });
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(self.private_key.as_bytes())
            .map_err(|e| Error::Config(format!("invalid GITHUB_APP_PRIVATE_KEY: {e}")))?;
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &key,
        )
        .map_err(|e| Error::Other(e.into()))?;
        *cache = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    /// POST /app/installations/{id}/access_tokens → fresh installation token.
    pub async fn installation_access_token(
        &self,
        installation_id: i64,
    ) -> Result<(String, DateTime<Utc>)> {
        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
            expires_at: String,
        }
        let resp: TokenResponse = self
            .http
            .post(format!(
                "{}/app/installations/{}/access_tokens",
                self.api_base, installation_id
            ))
            .bearer_auth(self.jwt()?)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let expires = DateTime::parse_from_rfc3339(&resp.expires_at)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now() + chrono::Duration::hours(1));
        Ok((resp.token, expires))
    }

    /// Load an installation row, refreshing its token when missing/expired.
    /// Port of GitHubInstallationService.get_or_refresh_installation.
    pub async fn installation_token(
        &self,
        db: &sqlx::PgPool,
        crypto: &Crypto,
        installation_id: i64,
    ) -> Result<String> {
        let row: Option<GithubInstallation> =
            sqlx::query_as("SELECT * FROM github_installation WHERE installation_id = $1")
                .bind(installation_id)
                .fetch_optional(db)
                .await?;

        let cached = row.as_ref().and_then(|r| {
            let valid = r
                .token_expires_at
                .map(|exp| exp > Utc::now() + chrono::Duration::minutes(1))
                .unwrap_or(false);
            if valid {
                r.token.clone()
            } else {
                None
            }
        });
        if let Some(enc) = cached {
            return crypto.decrypt(&enc);
        }

        let (token, expires) = self.installation_access_token(installation_id).await?;
        let enc = crypto.encrypt(&token)?;
        sqlx::query(
            "INSERT INTO github_installation (installation_id, token, token_expires_at, status)
             VALUES ($1, $2, $3, 'active')
             ON CONFLICT (installation_id) DO UPDATE
             SET token = $2, token_expires_at = $3",
        )
        .bind(installation_id)
        .bind(&enc)
        .bind(expires)
        .execute(db)
        .await?;
        Ok(token)
    }

    /// Repo metadata by numeric id (installation token).
    pub async fn repository(
        &self,
        installation_token: &str,
        repo_id: i64,
    ) -> Result<serde_json::Value> {
        Ok(self
            .http
            .get(format!("{}/repositories/{}", self.api_base, repo_id))
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Latest commit on a branch (installation token).
    pub async fn latest_commit(
        &self,
        installation_token: &str,
        repo_full_name: &str,
        branch: &str,
    ) -> Result<serde_json::Value> {
        Ok(self
            .http
            .get(format!(
                "{}/repos/{}/commits/{}",
                self.api_base, repo_full_name, branch
            ))
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Repos visible to the user token under an installation.
    pub async fn installation_repositories_for_user(
        &self,
        user_token: &str,
        installation_id: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let mut repos = vec![];
        let mut page = 1u32;
        loop {
            let data: serde_json::Value = self
                .http
                .get(format!(
                    "{}/user/installations/{}/repositories",
                    self.api_base, installation_id
                ))
                .bearer_auth(user_token)
                .query(&[("per_page", "100"), ("page", &page.to_string())])
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "runway")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let batch = data["repositories"].as_array().cloned().unwrap_or_default();
            let total = data["total_count"].as_i64().unwrap_or(0);
            let n = batch.len();
            repos.extend(batch);
            if n == 0 || (page as i64) * 100 >= total {
                break;
            }
            page += 1;
        }
        Ok(repos)
    }

    /// Installations the OAuth user can administer.
    pub async fn user_installations(&self, user_token: &str) -> Result<Vec<serde_json::Value>> {
        let data: serde_json::Value = self
            .http
            .get(format!("{}/user/installations", self.api_base))
            .bearer_auth(user_token)
            .query(&[("per_page", "100")])
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(data["installations"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    // -- OAuth login ---------------------------------------------------------

    pub async fn exchange_code(&self, code: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Token {
            access_token: Option<String>,
        }
        let resp: Token = self
            .http
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        resp.access_token
            .ok_or_else(|| Error::Config("github oauth exchange returned no token".into()))
    }

    /// (id, login, name) for the OAuth user.
    pub async fn user_info(&self, user_token: &str) -> Result<(i64, String, Option<String>)> {
        let v: serde_json::Value = self
            .http
            .get(format!("{}/user", self.api_base))
            .bearer_auth(user_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok((
            v["id"].as_i64().unwrap_or_default(),
            v["login"].as_str().unwrap_or_default().to_string(),
            v["name"].as_str().map(String::from),
        ))
    }

    pub async fn user_primary_email(&self, user_token: &str) -> Result<Option<String>> {
        let v: serde_json::Value = self
            .http
            .get(format!("{}/user/emails", self.api_base))
            .bearer_auth(user_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(v.as_array().and_then(|emails| {
            emails
                .iter()
                .find(|e| {
                    e["primary"].as_bool() == Some(true) && e["verified"].as_bool() == Some(true)
                })
                .and_then(|e| e["email"].as_str().map(String::from))
        }))
    }

    // -- Webhooks ------------------------------------------------------------

    /// Verify `X-Hub-Signature-256`. Constant-time compare.
    pub fn verify_webhook(secret: &str, payload: &[u8], signature_header: &str) -> bool {
        let Some(sig) = signature_header.strip_prefix("sha256=") else {
            return false;
        };
        let Ok(expected) = hex::decode(sig) else {
            return false;
        };
        let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(payload);
        mac.verify_slice(&expected).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, payload: &[u8]) -> String {
        use hmac::Mac;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn webhook_signature_ok() {
        let payload = b"{\"action\":\"opened\"}";
        let header = sign("s3cret", payload);
        assert!(GithubService::verify_webhook("s3cret", payload, &header));
    }

    #[test]
    fn webhook_signature_rejects_bad() {
        let payload = b"{}";
        assert!(!GithubService::verify_webhook(
            "s3cret",
            payload,
            "sha256=deadbeef"
        ));
        assert!(!GithubService::verify_webhook(
            "wrong",
            payload,
            &sign("s3cret", payload)
        ));
        assert!(!GithubService::verify_webhook(
            "s3cret",
            payload,
            "not-a-signature"
        ));
    }
}
