//! Persistent CLI config (~/.config/runway/config.json) and a thin
//! authenticated HTTP client over the Runway REST API.

use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CliConfig {
    /// Instance base URL, e.g. "https://runway.example.com".
    pub server: Option<String>,
    /// `ak_` API key from `runway bootstrap` or the dashboard.
    pub key: Option<String>,
}

/// Link file written by `runway link` into `<cwd>/.runway/project.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkFile {
    pub project_id: String,
    pub project_name: Option<String>,
}

fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("runway");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("runway")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> CliConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &CliConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

/// Resolve `path` (relative to CWD) for `.runway/project.json`.
pub fn link_path() -> PathBuf {
    PathBuf::from(".runway").join("project.json")
}

pub fn load_link() -> anyhow::Result<LinkFile> {
    let path = link_path();
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "no linked project ({}) — run `runway link <project-id>`",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).context("corrupt .runway/project.json")
}

pub fn save_link(link: &LinkFile) -> anyhow::Result<()> {
    let path = link_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(link)?)?;
    Ok(())
}

pub struct Client {
    pub base: String,
    key: String,
    http: reqwest::Client,
}

impl Client {
    /// Build from saved config; errors when `login` hasn't run.
    pub fn from_config() -> anyhow::Result<Self> {
        let cfg = load_config();
        let (Some(server), Some(key)) = (cfg.server, cfg.key) else {
            bail!("not logged in — run `runway login --server <url> --key <ak_...>`");
        };
        Ok(Self::new(&server, &key))
    }

    pub fn new(server: &str, key: &str) -> Self {
        Self {
            base: server.trim_end_matches('/').to_string(),
            key: key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.url(path))
            .bearer_auth(&self.key)
    }

    /// Decode an API error body (`{"error": "..."}`) or fall back to status.
    async fn decode(resp: reqwest::Response) -> anyhow::Result<Value> {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(serde_json::from_str(&body).unwrap_or(Value::Null));
        }
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .or_else(|| v.get("detail"))
                    .or_else(|| v.get("message"))
                    .and_then(|e| e.as_str().map(String::from))
            })
            .unwrap_or_else(|| body.chars().take(300).collect());
        bail!("{} {}: {}", status.as_u16(), status.as_str(), detail);
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<Value> {
        Self::decode(self.req(reqwest::Method::GET, path).send().await?).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        Self::decode(
            self.req(reqwest::Method::POST, path)
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        Self::decode(
            self.req(reqwest::Method::PATCH, path)
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn delete(&self, path: &str) -> anyhow::Result<Value> {
        Self::decode(self.req(reqwest::Method::DELETE, path).send().await?).await
    }

    /// Raw gzip body upload (deployment tarball).
    pub async fn upload(&self, path: &str, bytes: Vec<u8>) -> anyhow::Result<Value> {
        Self::decode(
            self.req(reqwest::Method::POST, path)
                .header("content-type", "application/gzip")
                .body(bytes)
                .send()
                .await?,
        )
        .await
    }

    /// Plain-text response (logs endpoint).
    pub async fn get_text(&self, path: &str) -> anyhow::Result<String> {
        let resp = self.req(reqwest::Method::GET, path).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{} {}: {}", status.as_u16(), status.as_str(), body);
        }
        Ok(body)
    }
}
