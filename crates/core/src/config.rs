//! Runtime settings, loaded from environment (and .env via dotenvy).
//! Ports the devpush `Settings` surface; RUNWAY_* names preferred,
//! DEVPUSH_* read as fallback for shared env conventions.

use std::path::PathBuf;

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
    /// Host-side data dir (for bind mounts when running inside Docker).
    pub host_data_dir: Option<String>,
    /// App install dir (repo root in dev, /opt/runway in prod).
    pub app_dir: String,
    /// Signing key for session JWTs.
    pub secret_key: String,
    /// AES-256-GCM key (base64 or passphrase) for secrets at rest.
    pub encryption_key: String,
    /// Public IP for DNS verification hints. Optional — tunnel mode needs none.
    pub server_ip: Option<String>,
    /// CPU quota per deployment (0 = unlimited).
    pub default_cpus: f64,
    /// Max CPU a project may request.
    pub max_cpus: f64,
    /// Memory limit per deployment in MB (0 = unlimited).
    pub default_memory_mb: i64,
    /// Max memory a project may request in MB.
    pub max_memory_mb: i64,
    /// Allow projects to override CPU/memory via config.
    pub allow_custom_cpu: bool,
    pub allow_custom_memory: bool,
    /// Seconds a deployment may stay non-ready before failing.
    pub deployment_timeout_seconds: u64,
    /// Interval between monitor probe ticks.
    pub monitor_interval_seconds: u64,
    /// Seconds before a stopped container is deleted.
    pub container_delete_grace_seconds: u64,
    /// Docker restart policy for runner containers.
    pub deployment_restart_policy: String,
    pub deployment_restart_max_retries: i64,
    /// Uid/gid for runner containers (injected as PUID/PGID).
    pub service_uid: u32,
    pub service_gid: u32,

    // GitHub App (repos + webhooks) — env creds are an alternative to
    // DB registration via the app-manifest flow.
    pub github_app_id: Option<String>,
    pub github_app_name: Option<String>,
    pub github_app_private_key: Option<String>,
    pub github_app_webhook_secret: Option<String>,
    /// GitHub API base — override for GitHub Enterprise
    /// (`https://ghe.example.com/api/v3`).
    pub github_api_url: String,
    /// Gitea/GitLab webhook secrets — verify X-Gitea-Signature /
    /// X-Gitlab-Token on inbound push events.
    pub gitea_webhook_secret: Option<String>,
    pub gitlab_webhook_secret: Option<String>,
    /// Optional Bitbucket webhook secret — Bitbucket doesn't sign
    /// payloads; when set, the hook URL must carry `?secret=`.
    pub bitbucket_webhook_secret: Option<String>,

    // Dedicated OAuth sign-in (github/google) — separate from the
    // GitHub App creds: these carry `read:user user:email` scopes and
    // exist purely for "Continue with …" login.
    pub github_oauth_client_id: Option<String>,
    pub github_oauth_client_secret: Option<String>,
    pub google_oauth_client_id: Option<String>,
    pub google_oauth_client_secret: Option<String>,

    // OIDC / SSO (login + account link)
    pub oidc_client_id: Option<String>,
    pub oidc_client_secret: Option<String>,
    /// e.g. `https://idp.example.com/.well-known/openid-configuration`
    pub oidc_discovery_url: Option<String>,
    pub oidc_display_name: String,

    /// Sign-up allowlist UX — message + optional webhook on denial.
    pub access_denied_message: String,
    pub access_denied_webhook: Option<String>,

    /// Session cookie name.
    pub session_cookie: String,
    /// Session lifetime in seconds.
    pub session_max_age: u64,

    // SMTP (magic link + notifications)
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub smtp_user: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
    pub smtp_tls: bool,

    /// Directory holding the built React SPA (`web/dist` in dev).
    pub web_dir: String,

    /// Instance-level Cloudflare API token (CGNAT path). Needs
    /// Zone.DNS + Account.Tunnel permissions.
    pub cf_api_token: Option<String>,
    pub cf_account_id: Option<String>,
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        // .env lives next to the data dir in prod; plain dotenv() covers cwd.
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_filename("./data/.env");

        let disable_tls = env_bool("DISABLE_TLS", false);
        let env_dev = env_or("RUNWAY_ENV", "") == "development";
        let url_scheme = if disable_tls || env_dev {
            "http"
        } else {
            "https"
        }
        .to_string();

        let data_dir = env_or("DATA_DIR", "./data");

        Ok(Self {
            app_hostname: env_req("APP_HOSTNAME")?,
            deploy_domain: env_req("DEPLOY_DOMAIN")?,
            url_scheme,
            disable_tls,
            database_url: env_req("DATABASE_URL")?,
            docker_host: env_or("DOCKER_HOST", "unix:///var/run/docker.sock"),
            listen_addr: env_or("RUNWAY_LISTEN", "0.0.0.0:8000"),
            host_data_dir: std::env::var("HOST_DATA_DIR")
                .ok()
                .filter(|s| !s.is_empty()),
            app_dir: env_or("APP_DIR", "."),
            data_dir,
            secret_key: env_req("SECRET_KEY")?,
            encryption_key: env_or("ENCRYPTION_KEY", ""),
            server_ip: std::env::var("SERVER_IP").ok().filter(|s| !s.is_empty()),
            default_cpus: env_f64("DEFAULT_CPUS", 0.25),
            max_cpus: env_f64("MAX_CPUS", 4.0),
            default_memory_mb: env_i64("DEFAULT_MEMORY_MB", 512),
            max_memory_mb: env_i64("MAX_MEMORY_MB", 8192),
            allow_custom_cpu: env_bool("ALLOW_CUSTOM_CPU", true),
            allow_custom_memory: env_bool("ALLOW_CUSTOM_MEMORY", true),
            deployment_timeout_seconds: env_u64("DEPLOYMENT_TIMEOUT_SECONDS", 300),
            monitor_interval_seconds: env_u64("MONITOR_INTERVAL_SECONDS", 2),
            container_delete_grace_seconds: env_u64("CONTAINER_DELETE_GRACE_SECONDS", 30),
            deployment_restart_policy: env_or("DEPLOYMENT_RESTART_POLICY", "always"),
            deployment_restart_max_retries: env_i64("DEPLOYMENT_RESTART_MAX_RETRIES", 5),
            service_uid: env_u64("SERVICE_UID", 1000) as u32,
            service_gid: env_u64("SERVICE_GID", 1000) as u32,
            github_app_id: opt("GITHUB_APP_ID"),
            github_app_name: opt("GITHUB_APP_NAME"),
            github_app_private_key: opt("GITHUB_APP_PRIVATE_KEY"),
            github_app_webhook_secret: opt("GITHUB_APP_WEBHOOK_SECRET"),
            github_api_url: env_or("GITHUB_API_URL", "https://api.github.com"),
            gitea_webhook_secret: opt("GITEA_WEBHOOK_SECRET"),
            gitlab_webhook_secret: opt("GITLAB_WEBHOOK_SECRET"),
            bitbucket_webhook_secret: opt("BITBUCKET_WEBHOOK_SECRET"),
            github_oauth_client_id: opt("GITHUB_OAUTH_CLIENT_ID"),
            github_oauth_client_secret: opt("GITHUB_OAUTH_CLIENT_SECRET"),
            google_oauth_client_id: opt("GOOGLE_CLIENT_ID"),
            google_oauth_client_secret: opt("GOOGLE_CLIENT_SECRET"),
            oidc_client_id: opt("OIDC_CLIENT_ID"),
            oidc_client_secret: opt("OIDC_CLIENT_SECRET"),
            oidc_discovery_url: opt("OIDC_DISCOVERY_URL"),
            oidc_display_name: env_or("OIDC_DISPLAY_NAME", "SSO"),
            access_denied_message: env_or(
                "ACCESS_DENIED_MESSAGE",
                "Sign-in not allowed for this email.",
            ),
            access_denied_webhook: opt("ACCESS_DENIED_WEBHOOK"),
            session_cookie: env_or("SESSION_COOKIE", "runway_session"),
            session_max_age: env_u64("SESSION_MAX_AGE", 60 * 60 * 24 * 30),
            smtp_host: opt("SMTP_HOST"),
            smtp_port: env_u64("SMTP_PORT", 587) as u16,
            smtp_user: opt("SMTP_USER"),
            smtp_password: opt("SMTP_PASSWORD"),
            smtp_from: opt("SMTP_FROM"),
            smtp_tls: env_bool("SMTP_TLS", true),
            web_dir: env_or("WEB_DIR", "./web/dist"),
            cf_api_token: opt("CF_API_TOKEN").or_else(|| opt("CLOUDFLARE_API_TOKEN")),
            cf_account_id: opt("CF_ACCOUNT_ID").or_else(|| opt("CLOUDFLARE_ACCOUNT_ID")),
        })
    }

    /// True when the instance-level Cloudflare Tunnel can be managed.
    pub fn cf_configured(&self) -> bool {
        self.cf_api_token.is_some() && self.cf_account_id.is_some()
    }

    /// True when the GitHub App integration is fully configured.
    pub fn github_app_configured(&self) -> bool {
        self.github_app_id.is_some()
            && self.github_app_private_key.is_some()
            && self.github_app_webhook_secret.is_some()
    }

    /// True when a dedicated OAuth sign-in provider is configured.
    pub fn oauth_configured(&self, provider: &str) -> bool {
        match provider {
            "github" => {
                self.github_oauth_client_id.is_some() && self.github_oauth_client_secret.is_some()
            }
            "google" => {
                self.google_oauth_client_id.is_some() && self.google_oauth_client_secret.is_some()
            }
            _ => false,
        }
    }

    /// True when OIDC SSO is fully configured.
    pub fn oidc_configured(&self) -> bool {
        self.oidc_client_id.is_some()
            && self.oidc_client_secret.is_some()
            && self.oidc_discovery_url.is_some()
    }

    pub fn traefik_dir(&self) -> PathBuf {
        PathBuf::from(&self.data_dir).join("traefik")
    }

    pub fn logs_dir(&self) -> PathBuf {
        PathBuf::from(&self.data_dir).join("logs")
    }
}

fn env_req(key: &str) -> Result<String> {
    opt(key).ok_or_else(|| Error::Config(format!("{key} is required")))
}

fn opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn env_or(key: &str, default: &str) -> String {
    opt(key).unwrap_or_else(|| default.to_string())
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

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
pub(crate) fn test_settings() -> Settings {
    Settings {
        app_hostname: "runway.test".into(),
        deploy_domain: "deploy.test".into(),
        url_scheme: "https".into(),
        disable_tls: false,
        database_url: "postgres://localhost/test".into(),
        docker_host: "unix:///var/run/docker.sock".into(),
        listen_addr: "0.0.0.0:8000".into(),
        data_dir: "./data".into(),
        host_data_dir: None,
        app_dir: ".".into(),
        secret_key: "test-secret".into(),
        encryption_key: "test-encryption-key".into(),
        server_ip: None,
        default_cpus: 0.25,
        max_cpus: 4.0,
        default_memory_mb: 512,
        max_memory_mb: 8192,
        allow_custom_cpu: true,
        allow_custom_memory: true,
        deployment_timeout_seconds: 300,
        monitor_interval_seconds: 2,
        container_delete_grace_seconds: 30,
        deployment_restart_policy: "always".into(),
        deployment_restart_max_retries: 5,
        service_uid: 1000,
        service_gid: 1000,
        github_app_id: None,
        github_app_name: None,
        github_app_private_key: None,
        github_app_webhook_secret: None,
        github_api_url: "https://api.github.com".into(),
        gitea_webhook_secret: None,
        gitlab_webhook_secret: None,
        bitbucket_webhook_secret: None,
        github_oauth_client_id: None,
        github_oauth_client_secret: None,
        google_oauth_client_id: None,
        google_oauth_client_secret: None,
        oidc_client_id: None,
        oidc_client_secret: None,
        oidc_discovery_url: None,
        oidc_display_name: "SSO".into(),
        access_denied_message: "Sign-in not allowed for this email.".into(),
        access_denied_webhook: None,
        session_cookie: "runway_session".into(),
        session_max_age: 2592000,
        smtp_host: None,
        smtp_port: 587,
        smtp_user: None,
        smtp_password: None,
        smtp_from: None,
        smtp_tls: true,
        web_dir: "./web/dist".into(),
        cf_api_token: None,
        cf_account_id: None,
    }
}
