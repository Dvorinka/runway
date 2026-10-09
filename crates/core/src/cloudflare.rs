//! Cloudflare API v4 client: DNS records, zones, and Tunnel management.
//!
//! Port of devpush `services/cloudflare.py`. Two access modes:
//! - Team-level: DNS assign + per-team tunnel for custom domains.
//! - Instance-level: one admin-managed tunnel covering APP_HOSTNAME +
//!   *.DEPLOY_DOMAIN — the CGNAT story.

use base64::Engine;
use serde_json::{json, Value};

use crate::error::Result;

pub const API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, Clone)]
pub struct CloudflareClient {
    token: String,
    http: reqwest::Client,
}

/// Result of `create_tunnel` — the token is returned only at creation.
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub id: String,
    pub name: String,
    pub token: String,
}

impl CloudflareClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Verify the token.
    pub async fn verify(&self) -> Result<Value> {
        let res = self
            .http
            .get(format!("{API_BASE}/user/tokens/verify"))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    /// `GET /accounts` — devpush picks the first account for a token.
    pub async fn list_accounts(&self) -> Result<Vec<Value>> {
        let res: Value = self
            .http
            .get(format!("{API_BASE}/accounts"))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res["result"].as_array().cloned().unwrap_or_default())
    }

    // -- Zones ---------------------------------------------------------

    pub async fn list_zones(&self) -> Result<Vec<Value>> {
        let mut zones = vec![];
        let mut page = 1u32;
        loop {
            let res: Value = self
                .http
                .get(format!("{API_BASE}/zones"))
                .bearer_auth(&self.token)
                .query(&[("page", page.to_string()), ("per_page", "50".into())])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let batch = res
                .get("result")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let len = batch.len();
            zones.extend(batch);
            if len < 50 {
                break;
            }
            page += 1;
        }
        Ok(zones)
    }

    /// Zone whose name is the longest suffix of `hostname` (apex-safe).
    pub async fn find_zone_for_hostname(&self, hostname: &str) -> Result<Option<Value>> {
        let mut zones = self.list_zones().await?;
        zones.sort_by_key(|z| {
            std::cmp::Reverse(z.get("name").and_then(|n| n.as_str()).unwrap_or("").len())
        });
        Ok(zones.into_iter().find(|z| {
            let name = z.get("name").and_then(|n| n.as_str()).unwrap_or("");
            hostname == name || hostname.ends_with(&format!(".{name}"))
        }))
    }

    // -- DNS -----------------------------------------------------------

    pub async fn create_dns_record(
        &self,
        zone_id: &str,
        record_type: &str,
        name: &str,
        content: &str,
        proxied: bool,
    ) -> Result<Option<Value>> {
        let res: Value = self
            .http
            .post(format!("{API_BASE}/zones/{zone_id}/dns_records"))
            .bearer_auth(&self.token)
            .json(&json!({
                "type": record_type,
                "name": name,
                "content": content,
                "proxied": proxied,
                "ttl": 1,
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(if res["success"].as_bool() == Some(true) {
            res.get("result").cloned()
        } else {
            None
        })
    }

    pub async fn list_dns_records(&self, zone_id: &str, name: Option<&str>) -> Result<Vec<Value>> {
        let mut records = vec![];
        let mut page = 1u32;
        loop {
            let mut q = vec![("per_page", "100".to_string()), ("page", page.to_string())];
            if let Some(n) = name {
                q.push(("name", n.to_string()));
            }
            let res: Value = self
                .http
                .get(format!("{API_BASE}/zones/{zone_id}/dns_records"))
                .bearer_auth(&self.token)
                .query(&q)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let batch = res
                .get("result")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let len = batch.len();
            records.extend(batch);
            if len < 100 {
                break;
            }
            page += 1;
        }
        Ok(records)
    }

    pub async fn delete_dns_record(&self, zone_id: &str, record_id: &str) -> Result<bool> {
        let res: Value = self
            .http
            .delete(format!(
                "{API_BASE}/zones/{zone_id}/dns_records/{record_id}"
            ))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res["success"].as_bool() == Some(true))
    }

    /// Create or update a DNS record pointing `hostname` at `target`.
    /// Apex → A record? No — port of the reference: apex uses A when the
    /// target is an IP shape; tunnel targets always use CNAME (CF
    /// flattens apex CNAMEs). Returns `{zone_id, record_id}`.
    pub async fn create_or_update_dns_record(
        &self,
        hostname: &str,
        target: &str,
        proxied: bool,
    ) -> Result<Option<Value>> {
        let Some(zone) = self.find_zone_for_hostname(hostname).await? else {
            return Ok(None);
        };
        let zone_id = zone["id"].as_str().unwrap_or("").to_string();
        let zone_name = zone["name"].as_str().unwrap_or("").to_string();
        let is_apex = hostname == zone_name && target.parse::<std::net::IpAddr>().is_ok();
        let record_type = if is_apex { "A" } else { "CNAME" };

        let existing = self
            .list_dns_records(&zone_id, Some(hostname))
            .await?
            .into_iter()
            .find(|r| {
                r["type"].as_str() == Some(record_type) && r["name"].as_str() == Some(hostname)
            });

        if let Some(existing) = existing {
            let record_id = existing["id"].as_str().unwrap_or("").to_string();
            let res: Value = self
                .http
                .put(format!(
                    "{API_BASE}/zones/{zone_id}/dns_records/{record_id}"
                ))
                .bearer_auth(&self.token)
                .json(&json!({
                    "type": record_type,
                    "name": hostname,
                    "content": target,
                    "proxied": proxied,
                    "ttl": 1,
                }))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            return Ok(if res["success"].as_bool() == Some(true) {
                Some(json!({ "zone_id": zone_id, "record_id": record_id }))
            } else {
                None
            });
        }

        Ok(self
            .create_dns_record(&zone_id, record_type, hostname, target, proxied)
            .await?
            .map(|r| json!({ "zone_id": zone_id, "record_id": r["id"] })))
    }

    // -- Tunnels --------------------------------------------------------

    /// Create a remotely-managed tunnel. Token only returned here.
    pub async fn create_tunnel(&self, account_id: &str, name: &str) -> Result<Option<TunnelInfo>> {
        let secret = base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>());
        let res: Value = self
            .http
            .post(format!("{API_BASE}/accounts/{account_id}/cfd_tunnel"))
            .bearer_auth(&self.token)
            .json(&json!({
                "name": name,
                "tunnel_secret": secret,
                "config_src": "cloudflare",
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if res["success"].as_bool() != Some(true) {
            return Ok(None);
        }
        let result = &res["result"];
        let (id, token) = (
            result["id"].as_str().unwrap_or("").to_string(),
            result["token"].as_str().unwrap_or("").to_string(),
        );
        if id.is_empty() || token.is_empty() {
            return Ok(None);
        }
        Ok(Some(TunnelInfo {
            id,
            name: result["name"].as_str().unwrap_or(name).to_string(),
            token,
        }))
    }

    /// List remotely-managed tunnels, optionally filtered by name.
    pub async fn list_tunnels(&self, account_id: &str, name: Option<&str>) -> Result<Vec<Value>> {
        let mut q = vec![("per_page", "100".to_string())];
        if let Some(n) = name {
            q.push(("name", n.to_string()));
        }
        let res: Value = self
            .http
            .get(format!("{API_BASE}/accounts/{account_id}/cfd_tunnel"))
            .bearer_auth(&self.token)
            .query(&q)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res["result"].as_array().cloned().unwrap_or_default())
    }

    pub async fn get_tunnel(&self, account_id: &str, tunnel_id: &str) -> Result<Option<Value>> {
        let res = self
            .http
            .get(format!(
                "{API_BASE}/accounts/{account_id}/cfd_tunnel/{tunnel_id}"
            ))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: Value = res.error_for_status()?.json().await?;
        Ok(body.get("result").cloned())
    }

    pub async fn delete_tunnel(&self, account_id: &str, tunnel_id: &str) -> Result<bool> {
        let res: Value = self
            .http
            .delete(format!(
                "{API_BASE}/accounts/{account_id}/cfd_tunnel/{tunnel_id}"
            ))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res["success"].as_bool() == Some(true))
    }

    /// Current remotely-managed ingress config.
    pub async fn get_tunnel_config(
        &self,
        account_id: &str,
        tunnel_id: &str,
    ) -> Result<Option<Value>> {
        let res = self
            .http
            .get(format!(
                "{API_BASE}/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations"
            ))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: Value = res.error_for_status()?.json().await?;
        Ok(body.get("result").cloned())
    }

    /// Replace the tunnel ingress config: every hostname → `service_url`
    /// (Traefik), then the mandatory `http_status:404` catch-all.
    pub async fn update_tunnel_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostnames: &[String],
        service_url: &str,
    ) -> Result<bool> {
        let mut ingress: Vec<Value> = hostnames
            .iter()
            .map(|h| json!({ "hostname": h, "service": service_url }))
            .collect();
        ingress.push(json!({ "service": "http_status:404" }));
        let res: Value = self
            .http
            .put(format!(
                "{API_BASE}/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations"
            ))
            .bearer_auth(&self.token)
            .json(&json!({ "config": { "ingress": ingress } }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(res["success"].as_bool() == Some(true))
    }
}
