//! Cloudflare API v4 client: DNS records, zones, and Tunnel management.
//!
//! Two access modes, ported from the reference implementation:
//! - Team-level: DNS assign + per-team tunnel for custom domains.
//! - Instance-level (new): one admin-managed tunnel covering
//!   APP_HOSTNAME + *.DEPLOY_DOMAIN — the CGNAT story.
//!
//! Logs for deployments are file-tailed by the worker; no Loki needed.

use crate::error::Result;

pub const API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Clone)]
pub struct CloudflareClient {
    token: String,
    http: reqwest::Client,
}

impl CloudflareClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Verify the token. (Phase 3: full zone/DNS/tunnel surface.)
    pub async fn verify(&self) -> Result<serde_json::Value> {
        let res = self
            .http
            .get(format!("{API_BASE}/user/tokens/verify"))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }
}
