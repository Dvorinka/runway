//! GitHub client: App JWT (RS256), installation tokens, repo lookups,
//! app-manifest registration, webhook signature verification.
//! Port of devpush services/github.py.
//!
//! App credentials resolve from `GITHUB_APP_*` env vars or the
//! DB-registered `github_app` row (manifest flow). The service is always
//! constructible; `configured()` reports whether credentials are present,
//! and registration hot-patches them at runtime.

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::config::Settings;
use crate::crypto::Crypto;
use crate::error::{Error, Result};
use crate::models::{GithubApp, GithubInstallation};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct AppCreds {
    app_id: String,
    private_key: String,
    slug: Option<String>,
    webhook_secret: Option<String>,
}

/// Credentials returned by GitHub's app-manifest conversion endpoint.
#[derive(Debug, Deserialize)]
pub struct ManifestCreds {
    pub id: serde_json::Value,
    pub slug: String,
    pub name: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub pem: String,
    pub webhook_secret: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Clone)]
pub struct GithubService {
    http: reqwest::Client,
    pub api_base: String,
    pub web_base: String,
    creds: Arc<RwLock<Option<AppCreds>>>,
    jwt_cache: Arc<Mutex<Option<(String, Instant)>>>,
}

impl GithubService {
    /// App-level service. Always returns an instance — credentials are
    /// seeded from env when `github_app_configured()`, otherwise populated
    /// later via `load_from_db` or `configure`.
    pub fn from_settings(settings: &Settings) -> Self {
        let api_base = settings.github_api_url.trim_end_matches('/').to_string();
        let web_base = if api_base == "https://api.github.com" {
            "https://github.com".into()
        } else {
            api_base.trim_end_matches("/api/v3").to_string()
        };
        let creds = settings.github_app_configured().then(|| AppCreds {
            app_id: settings.github_app_id.clone().unwrap_or_default(),
            private_key: settings.github_app_private_key.clone().unwrap_or_default(),
            slug: settings.github_app_name.clone(),
            webhook_secret: settings.github_app_webhook_secret.clone(),
        });
        Self {
            http: reqwest::Client::new(),
            api_base,
            web_base,
            creds: Arc::new(RwLock::new(creds)),
            jwt_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Populate credentials from the DB-registered `github_app` row.
    /// Env-configured credentials always win — explicit ops config is
    /// authoritative and not shadowed by a registered app.
    pub async fn load_from_db(&self, db: &sqlx::PgPool, crypto: &Crypto) -> Result<()> {
        if self.configured() {
            return Ok(());
        }
        let row: Option<GithubApp> = sqlx::query_as(
            "SELECT app_id, slug, name, client_id, client_secret_enc, pem_enc,
                    webhook_secret_enc, html_url, created_by_user_id,
                    created_at, updated_at
             FROM github_app WHERE id = 1",
        )
        .fetch_optional(db)
        .await?;
        let Some(app) = row else { return Ok(()) };
        self.configure(RegisteredApp {
            app_id: app.app_id,
            slug: app.slug,
            private_key: crypto.decrypt(&app.pem_enc)?,
            webhook_secret: crypto.decrypt(&app.webhook_secret_enc)?,
        });
        Ok(())
    }

    /// Hot-patch app credentials (post-registration). Shared clones see
    /// the update immediately — no restart required.
    pub fn configure(&self, app: RegisteredApp) {
        *self.creds.write().unwrap() = Some(AppCreds {
            app_id: app.app_id,
            private_key: app.private_key,
            slug: Some(app.slug),
            webhook_secret: Some(app.webhook_secret),
        });
        *self.jwt_cache.lock().unwrap() = None;
    }

    pub fn configured(&self) -> bool {
        self.creds.read().unwrap().is_some()
    }

    /// `Some(&self)` when configured — preserves the old Option ergonomics
    /// at call sites that pass `Option<&GithubService>`.
    pub fn if_configured(&self) -> Option<&Self> {
        self.configured().then_some(self)
    }

    pub fn slug(&self) -> Option<String> {
        self.creds
            .read()
            .unwrap()
            .as_ref()
            .and_then(|c| c.slug.clone())
    }

    /// `https://github.com/apps/<slug>/installations/new`.
    pub fn install_url(&self) -> Option<String> {
        self.slug()
            .map(|s| format!("{}/apps/{s}/installations/new", self.web_base))
    }

    pub fn webhook_secret(&self) -> Option<String> {
        self.creds
            .read()
            .unwrap()
            .as_ref()
            .and_then(|c| c.webhook_secret.clone())
    }

    /// App JWT, cached for ~10 minutes (port of jwt_token property).
    pub fn jwt(&self) -> Result<String> {
        let (app_id, private_key) = {
            let creds = self.creds.read().unwrap();
            let Some(c) = creds.as_ref() else {
                return Err(Error::Config("GitHub App is not configured".into()));
            };
            (c.app_id.clone(), c.private_key.clone())
        };
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
            "iss": app_id,
        });
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key.as_bytes())
            .map_err(|e| Error::Config(format!("invalid app private key: {e}")))?;
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

    /// Installations of this app (`GET /app/installations`, app JWT).
    /// Replaces per-user OAuth listing — the instance sees every install.
    pub async fn app_installations(&self) -> Result<Vec<serde_json::Value>> {
        let mut installs = vec![];
        let mut page = 1u32;
        loop {
            let data: serde_json::Value = self
                .http
                .get(format!("{}/app/installations", self.api_base))
                .bearer_auth(self.jwt()?)
                .query(&[("per_page", "100"), ("page", &page.to_string())])
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "runway")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let batch = data.as_array().cloned().unwrap_or_default();
            let n = batch.len();
            installs.extend(batch);
            if n < 100 {
                break;
            }
            page += 1;
        }
        Ok(installs)
    }

    /// Repos under an installation (`GET /installation/repositories`,
    /// installation token). Paginated.
    pub async fn installation_repositories(
        &self,
        installation_token: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let mut repos = vec![];
        let mut page = 1u32;
        loop {
            let data: serde_json::Value = self
                .http
                .get(format!("{}/installation/repositories", self.api_base))
                .bearer_auth(installation_token)
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

    /// Recursive file path list for a branch (installation token).
    /// Feeds preset detection.
    pub async fn repo_files(
        &self,
        installation_token: &str,
        repo_full_name: &str,
        branch: &str,
    ) -> Result<Vec<String>> {
        let resp: serde_json::Value = self
            .http
            .get(format!(
                "{}/repos/{}/git/trees/{}?recursive=1",
                self.api_base, repo_full_name, branch
            ))
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .get("tree")
            .and_then(|t| t.as_array())
            .map(|t| {
                t.iter()
                    .filter_map(|e| e.get("path").and_then(|p| p.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Decoded text of a file at path (installation token). None on 404.
    pub async fn file_text(
        &self,
        installation_token: &str,
        repo_full_name: &str,
        path: &str,
    ) -> Result<Option<String>> {
        use base64::Engine;
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/contents/{}",
                self.api_base, repo_full_name, path
            ))
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: serde_json::Value = resp.error_for_status()?.json().await?;
        let Some(b64) = body.get("content").and_then(|c| c.as_str()) else {
            return Ok(None);
        };
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.replace('\n', ""))
            .map_err(|e| Error::Other(e.into()))?;
        Ok(Some(String::from_utf8_lossy(&raw).to_string()))
    }

    /// Commit status (installation token) — PR preview check on the commit.
    pub async fn commit_status(
        &self,
        installation_token: &str,
        repo_full_name: &str,
        sha: &str,
        state: &str,
        target_url: Option<&str>,
        description: &str,
    ) -> Result<()> {
        self.http
            .post(format!(
                "{}/repos/{}/statuses/{}",
                self.api_base, repo_full_name, sha
            ))
            .bearer_auth(installation_token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .json(&serde_json::json!({
                "state": state,
                "target_url": target_url,
                "description": description,
                "context": "runway/deploy",
            }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    // -- App manifest registration -------------------------------------------

    /// POST /app-manifests/{code}/conversions — exchange the code GitHub
    /// sends after the user confirms app creation. Unauthenticated.
    pub async fn exchange_manifest_code(&self, code: &str) -> Result<ManifestCreds> {
        Ok(self
            .http
            .post(format!(
                "{}/app-manifests/{}/conversions",
                self.api_base, code
            ))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "runway")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
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

/// Decrypted app credentials ready for `configure`.
pub struct RegisteredApp {
    pub app_id: String,
    pub slug: String,
    pub private_key: String,
    pub webhook_secret: String,
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

    #[test]
    fn unconfigured_service_reports_unconfigured() {
        let settings = crate::config::test_settings();
        let gh = GithubService::from_settings(&settings);
        assert!(!gh.configured());
        assert!(gh.if_configured().is_none());
        assert!(gh.jwt().is_err());
    }
}
