//! Runtime settings, loaded from environment (and .env via dotenvy).

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Settings {
    /// Hostname serving the dashboard/API, e.g. `runway.example.com`.
    pub app_hostname: String,
    /// Wildcard root for deployment URLs, e.g. `deploy.example.com`.
    pub deploy_domain: String,
    /// `http` or `https`. `https` unless DISABLE_TLS or dev mode.
    pub url_scheme: String,
    /// TLS terminated upstream (Cloudflare edge, external LB).
    pub disable_tls: bool,
    /// Postgres connection string.
    pub database_url: String,
    /// Docker API endpoint, e.g. `unix:///var/run/docker.sock` or `tcp://docker-proxy:2375`.
    pub docker_host: String,
    /// Host bind for the HTTP server.
    pub listen_addr: String,
    /// Data directory (logs, traefik dynamic conf, uploads, static artifacts).
    pub data_dir: String,
    /// Fernet-equivalent secret for encrypting stored secrets. (design TBD)
    pub secret_key: String,
    /// Public IP for DNS verification hints. Optional — tunnel mode needs none.
    pub server_ip: Option<String>,
    /// Seconds a deployment may stay non-ready before failing.
    pub deployment_timeout_seconds: u64,
    /// Interval between monitor probe ticks.
    pub monitor_interval_seconds: u64,
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let disable_tls = env_bool("DISABLE_TLS", false);
        let env_dev = std::env::var("RUNWAY_ENV").unwrap_or_default() == "development";
        let url_scheme = if disable_tls || env_dev {
            "http"
        } else {
            "https"
        }
        .to_string();

        Ok(Self {
            app_hostname: env_req("APP_HOSTNAME")?,
            deploy_domain: env_req("DEPLOY_DOMAIN")?,
            url_scheme,
            disable_tls,
            database_url: env_req("DATABASE_URL")?,
            docker_host: std::env::var("DOCKER_HOST")
                .unwrap_or_else(|_| "unix:///var/run/docker.sock".into()),
            listen_addr: std::env::var("RUNWAY_LISTEN").unwrap_or_else(|_| "0.0.0.0:8000".into()),
            data_dir: std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()),
            secret_key: env_req("SECRET_KEY")?,
            server_ip: std::env::var("SERVER_IP").ok().filter(|s| !s.is_empty()),
            deployment_timeout_seconds: env_u64("DEPLOYMENT_TIMEOUT_SECONDS", 300),
            monitor_interval_seconds: env_u64("MONITOR_INTERVAL_SECONDS", 2),
        })
    }
}

fn env_req(key: &str) -> Result<String> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Config(format!("{key} is required")))
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
