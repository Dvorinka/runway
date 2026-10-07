//! Core domain models. Rows map 1:1 to migrations/0001_init.sql, which
//! ports the devpush reference (`app/models.py`). JSONB `config` and
//! `environments` stay as `serde_json::Value` for parity; helpers below
//! reproduce the reference accessor semantics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

use crate::config::Settings;
use crate::crypto::Crypto;
use crate::error::{Error, Result};
use crate::slugify::{slugify, token_hex};

// ---------------------------------------------------------------------------
// User / identity / team
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub username: String,
    pub name: Option<String>,
    pub email_verified: bool,
    pub status: String,
    pub tokens_invalid_before: Option<DateTime<Utc>>,
    pub default_team_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct UserIdentity {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub provider_user_id: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub provider_metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub slug: Option<String>,
    pub has_avatar: bool,
    pub status: String,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Team {
    pub fn new(name: &str, created_by: i64) -> Self {
        let now = Utc::now();
        Self {
            id: token_hex(16),
            name: name.to_string(),
            slug: None,
            has_avatar: false,
            status: "active".into(),
            created_by_user_id: Some(created_by),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TeamMember {
    pub id: i64,
    pub team_id: String,
    pub user_id: i64,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GithubInstallation {
    pub installation_id: i64,
    /// AES-GCM ciphertext.
    pub token: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

/// One environment inside `project.environments` JSONB.
/// Port of the dict shape devpush stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// Branch pattern: exact name or `*` wildcard.
    #[serde(default)]
    pub branch: String,
    #[serde(default = "env_status_default")]
    pub status: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

fn env_status_default() -> String {
    "active".into()
}

impl Environment {
    pub fn production(branch: &str) -> Self {
        Self {
            id: "prod".into(),
            name: "Production".into(),
            slug: "production".into(),
            branch: branch.to_string(),
            status: "active".into(),
            extra: Default::default(),
        }
    }
}

/// One env var entry inside the encrypted `env_vars` JSON array.
/// `{"key": ..., "value": ..., "environment": <slug>|null}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Project {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: String,
    pub repo_provider: String,
    pub repo_id: Option<i64>,
    pub repo_full_name: String,
    pub repo_base_url: String,
    pub repo_branch: String,
    pub repo_status: String,
    pub github_installation_id: Option<i64>,
    pub gitea_connection_id: Option<i64>,
    pub gitlab_connection_id: Option<i64>,
    pub config: Value,
    pub environments: Value,
    /// AES-GCM ciphertext holding a `Vec<EnvVar>` JSON array.
    #[serde(skip)]
    pub env_vars: String,
    pub status: String,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn environments(&self) -> Vec<Environment> {
        serde_json::from_value(self.environments.clone()).unwrap_or_default()
    }

    pub fn active_environments(&self) -> Vec<Environment> {
        self.environments()
            .into_iter()
            .filter(|e| e.status == "active")
            .collect()
    }

    pub fn environment_by_id(&self, id: &str) -> Option<Environment> {
        self.environments().into_iter().find(|e| e.id == id)
    }

    /// Highest-priority environment matching a branch.
    /// Port of utils/environment.py:get_environment_for_branch — production
    /// (first entry) matches exactly; later entries support `*` wildcards.
    pub fn environment_for_branch(&self, branch: &str) -> Option<Environment> {
        let envs = self.active_environments();
        if envs.first().map(|e| e.branch.as_str()) == Some(branch) {
            return envs.into_iter().next();
        }
        for env in envs.into_iter().skip(1) {
            let pattern = env.branch.as_str();
            if pattern == branch {
                return Some(env);
            }
            if let Some(suffix) = pattern.strip_prefix('*') {
                if branch.ends_with(suffix) {
                    return Some(env);
                }
            } else if let Some(prefix) = pattern.strip_suffix('*') {
                if branch.starts_with(prefix) {
                    return Some(env);
                }
            } else if pattern.contains('*') {
                let (prefix, suffix) = pattern.split_once('*').unwrap();
                if branch.starts_with(prefix) && branch.ends_with(suffix) {
                    return Some(env);
                }
            }
        }
        None
    }

    /// Decrypted env var list.
    pub fn env_vars(&self, crypto: &Crypto) -> Result<Vec<EnvVar>> {
        if self.env_vars.is_empty() {
            return Ok(vec![]);
        }
        let json = crypto.decrypt(&self.env_vars)?;
        serde_json::from_str(&json).map_err(|e| Error::Crypto(e.to_string()))
    }

    /// Env vars flattened for one environment: globals first, then
    /// environment-scoped entries override same-named globals.
    /// Port of Project.get_env_vars.
    pub fn env_vars_for(&self, crypto: &Crypto, env_slug: &str) -> Result<Vec<EnvVar>> {
        let all = self.env_vars(crypto)?;
        let mut flat: Vec<EnvVar> = all
            .iter()
            .filter(|v| v.environment.is_none())
            .cloned()
            .collect();
        for var in all
            .iter()
            .filter(|v| v.environment.as_deref() == Some(env_slug))
        {
            flat.retain(|v| v.key != var.key);
            flat.push(var.clone());
        }
        Ok(flat)
    }

    pub fn set_env_vars(&mut self, crypto: &Crypto, vars: &[EnvVar]) -> Result<()> {
        self.env_vars = crypto.encrypt(&serde_json::to_string(vars)?)?;
        Ok(())
    }

    pub fn hostname(&self, settings: &Settings) -> String {
        format!(
            "{}.{}",
            self.slug.as_deref().unwrap_or(&self.id),
            settings.deploy_domain
        )
    }

    pub fn url(&self, settings: &Settings) -> String {
        format!("{}://{}", settings.url_scheme, self.hostname(settings))
    }

    pub fn environment_hostname(&self, env_slug: &str, settings: &Settings) -> String {
        if env_slug == "production" {
            return self.hostname(settings);
        }
        format!(
            "{}-env-{}.{}",
            self.slug.as_deref().unwrap_or(&self.id),
            env_slug,
            settings.deploy_domain
        )
    }

    /// Generate the project slug — port of the after_insert listener.
    pub async fn assign_slug(&mut self, db: &sqlx::PgPool, team_slug: &str) -> Result<()> {
        if self.slug.is_some() {
            return Ok(());
        }
        let mut base = slugify(&format!("{}-{}", self.name, team_slug), 40);
        if base.is_empty() {
            base = slugify(&format!("project-{}", self.id), 40);
        }
        let exists: Option<(String,)> = sqlx::query_as("SELECT slug FROM project WHERE slug = $1")
            .bind(&base)
            .fetch_optional(db)
            .await?;
        self.slug = Some(if exists.is_none() {
            base
        } else {
            slugify(
                &format!("{}-{}", &base[..base.len().min(32)], &self.id[..7]),
                40,
            )
        });
        sqlx::query("UPDATE project SET slug = $1 WHERE id = $2")
            .bind(&self.slug)
            .bind(&self.id)
            .execute(db)
            .await?;
        Ok(())
    }

    /// Generate the team slug — port of the after_insert listener.
    pub async fn assign_team_slug(db: &sqlx::PgPool, team: &Team) -> Result<String> {
        if let Some(slug) = &team.slug {
            return Ok(slug.clone());
        }
        let mut base = slugify(&team.name, 40);
        if base.is_empty() {
            base = format!("team-{}", team.id);
            base.truncate(40);
        }
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT slug FROM team WHERE lower(slug) = lower($1)")
                .bind(&base)
                .fetch_optional(db)
                .await?;
        let slug = if exists.is_none() {
            base
        } else {
            slugify(
                &format!("{}-{}", &base[..base.len().min(32)], &team.id[..7]),
                40,
            )
        };
        sqlx::query("UPDATE team SET slug = $1 WHERE id = $2")
            .bind(&slug)
            .bind(&team.id)
            .execute(db)
            .await?;
        Ok(slug)
    }
}

// ---------------------------------------------------------------------------
// Deployment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub repo_provider: String,
    pub repo_id: i64,
    pub repo_full_name: String,
    pub repo_base_url: String,
    pub environment_id: String,
    pub branch: String,
    pub commit_sha: String,
    pub commit_meta: Value,
    pub config: Value,
    pub image: Option<String>,
    /// AES-GCM ciphertext holding a `Vec<EnvVar>` JSON array snapshot.
    #[serde(skip)]
    pub env_vars: String,
    pub job_id: Option<String>,
    pub error: Option<Value>,
    pub container_id: Option<String>,
    pub container_status: Option<String>,
    pub observed_status: Option<String>,
    pub observed_exit_code: Option<i32>,
    pub observed_at: Option<DateTime<Utc>>,
    pub observed_reason: Option<String>,
    pub observed_last_seen_at: Option<DateTime<Utc>>,
    pub observed_missing_count: i32,
    pub status: String,
    pub conclusion: Option<String>,
    pub trigger: String,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub concluded_at: Option<DateTime<Utc>>,
}

impl Deployment {
    /// `{project_slug}-id-{id[:7]}` — the immutable deployment subdomain.
    pub fn slug(&self, project_slug: &str) -> String {
        format!("{project_slug}-id-{}", &self.id[..7])
    }

    pub fn hostname(&self, project_slug: &str, settings: &Settings) -> String {
        format!("{}.{}", self.slug(project_slug), settings.deploy_domain)
    }

    pub fn url(&self, project_slug: &str, settings: &Settings) -> String {
        format!(
            "{}://{}",
            settings.url_scheme,
            self.hostname(project_slug, settings)
        )
    }

    /// Port of Deployment.computed_status.
    pub fn computed_status(&self) -> String {
        let observed = self.observed_status.as_deref();
        let expected = self.container_status.as_deref();
        match expected {
            Some("stopped") | Some("removed") => {
                if observed == Some("running") {
                    "orphaned".into()
                } else {
                    expected.unwrap().to_string()
                }
            }
            Some("running") => match observed {
                Some("not_found") => "missing".into(),
                Some("exited") => {
                    if self.observed_exit_code == Some(0) {
                        "stopped".into()
                    } else {
                        "crashed".into()
                    }
                }
                Some(o @ ("paused" | "dead")) => o.into(),
                _ => "running".into(),
            },
            _ => match observed {
                Some("exited") => {
                    if self.observed_exit_code == Some(0) {
                        "stopped".into()
                    } else {
                        "crashed".into()
                    }
                }
                Some(o) => o.into(),
                None => expected.unwrap_or("unknown").into(),
            },
        }
    }

    pub fn deployment_port(&self) -> i64 {
        self.config
            .get("port")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
            .unwrap_or(8000)
    }

    /// True when the deployment materializes to a static artifact
    /// (config.output_directory) rather than a long-lived process.
    pub fn is_static(&self) -> bool {
        self.config
            .get("output_directory")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
    }

    /// Port the serving container listens on. Static deploys are served
    /// by the `static-web` image on :80.
    pub fn serve_port(&self) -> i64 {
        if self.is_static() {
            80
        } else {
            self.deployment_port()
        }
    }

    pub fn env_vars(&self, crypto: &Crypto) -> Result<Vec<EnvVar>> {
        if self.env_vars.is_empty() {
            return Ok(vec![]);
        }
        let json = crypto.decrypt(&self.env_vars)?;
        serde_json::from_str(&json).map_err(|e| Error::Crypto(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Alias / domain / tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Alias {
    pub id: i64,
    pub subdomain: String,
    pub deployment_id: String,
    pub previous_deployment_id: Option<String>,
    pub r#type: String,
    pub value: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl Alias {
    pub fn hostname(&self, settings: &Settings) -> String {
        format!("{}.{}", self.subdomain, settings.deploy_domain)
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Domain {
    pub id: i64,
    pub project_id: String,
    pub hostname: String,
    pub r#type: String,
    pub environment_id: Option<String>,
    pub status: String,
    pub message: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub cloudflare_zone_id: Option<String>,
    pub cloudflare_record_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DeployToken {
    pub id: String,
    pub project_id: String,
    pub name: String,
    /// sha256 hex of the raw token.
    #[serde(skip)]
    pub token: String,
    pub environment_id: Option<String>,
    pub status: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl DeployToken {
    pub fn generate() -> (String, String) {
        let raw = format!("rw_{}", token_hex(32));
        let hash = crate::crypto::sha256_hex(&raw);
        (raw, hash)
    }

    pub fn can_deploy_environment(&self, environment_id: &str) -> bool {
        self.environment_id
            .as_deref()
            .is_none_or(|e| e == environment_id)
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ApiKey {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    /// sha256 hex of the raw token.
    #[serde(skip)]
    pub token: String,
    pub status: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApiKey {
    pub fn generate() -> (String, String) {
        let raw = format!("ak_{}", token_hex(32));
        let hash = crate::crypto::sha256_hex(&raw);
        (raw, hash)
    }
}

// ---------------------------------------------------------------------------
// Webhooks (Phase 4 — ports devpush team_webhook / project_webhook)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TeamWebhook {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub url: String,
    /// AES-GCM ciphertext; HMAC signing secret when set.
    #[serde(skip)]
    pub secret: Option<String>,
    pub events: Value,
    /// JSON array of project ids; NULL = all projects in the team.
    pub project_ids: Option<Value>,
    pub status: String,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TeamWebhook {
    /// Port of TeamWebhook.applies_to_project — NULL list means all projects.
    pub fn applies_to_project(&self, project_id: &str) -> bool {
        match &self.project_ids {
            None => true,
            Some(ids) => ids
                .as_array()
                .map(|a| a.iter().any(|v| v.as_str() == Some(project_id)))
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProjectWebhook {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub url: String,
    /// AES-GCM ciphertext; HMAC signing secret when set.
    #[serde(skip)]
    pub secret: Option<String>,
    pub events: Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Cloudflare connection (per-team) — port of devpush cloudflare_connection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CloudflareConnection {
    pub id: String,
    pub team_id: String,
    pub account_id: String,
    pub account_name: String,
    pub auth_method: String,
    /// AES-GCM ciphertext.
    #[serde(skip)]
    pub api_token: String,
    #[serde(skip)]
    pub oauth_refresh_token: Option<String>,
    pub oauth_expires_at: Option<DateTime<Utc>>,
    pub tunnel_id: Option<String>,
    pub tunnel_name: Option<String>,
    /// AES-GCM ciphertext.
    #[serde(skip)]
    pub tunnel_token: Option<String>,
    pub tunnel_container_id: Option<String>,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CloudflareConnection {
    pub fn has_tunnel(&self) -> bool {
        self.tunnel_id.is_some() && self.tunnel_token.is_some()
    }
    pub fn api_token_dec(&self, crypto: &Crypto) -> Result<String> {
        crypto.decrypt(&self.api_token)
    }
    pub fn tunnel_token_dec(&self, crypto: &Crypto) -> Result<Option<String>> {
        self.tunnel_token
            .as_deref()
            .map(|t| crypto.decrypt(t))
            .transpose()
    }
}

// ---------------------------------------------------------------------------
// Storage — port of devpush storage + storage_project (~830)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Storage {
    pub id: String,
    pub name: String,
    /// database | volume | kv | queue
    pub r#type: String,
    /// pending | active | resetting | error | deleted
    pub status: String,
    pub config: Value,
    pub error: Option<Value>,
    pub team_id: String,
    pub created_by_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Storage {
    pub fn engine(&self) -> &str {
        self.config["engine"].as_str().unwrap_or("sqlite")
    }
    pub fn container_name(&self) -> String {
        format!("storage-{}", &self.id[..12])
    }
    pub fn network_name(&self) -> String {
        format!("runway_storage_{}", &self.id[..12])
    }
    /// `data/storage/<team>/<type-dir>/<name>` — type-dir is `database`
    /// for database, `kv` for kv, `volume` for volume (devpush parity).
    pub fn data_dir(&self, data_dir: &str) -> String {
        let type_dir = match self.r#type.as_str() {
            "database" => "database",
            "kv" => "kv",
            _ => "volume",
        };
        format!(
            "{data_dir}/storage/{}/{type_dir}/{}",
            self.team_id, self.name
        )
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StorageProject {
    pub id: String,
    pub storage_id: String,
    pub project_id: String,
    pub environment_ids: Option<Value>,
    pub secrets: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Job queue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub payload: Value,
    pub run_at: DateTime<Utc>,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project_with_envs(envs: Value) -> Project {
        let now = Utc::now();
        Project {
            id: "p1".into(),
            team_id: "t1".into(),
            name: "proj".into(),
            slug: Some("proj".into()),
            description: String::new(),
            repo_provider: "github".into(),
            repo_id: Some(1),
            repo_full_name: "o/r".into(),
            repo_base_url: "https://github.com".into(),
            repo_branch: "main".into(),
            repo_status: "active".into(),
            github_installation_id: None,
            gitea_connection_id: None,
            gitlab_connection_id: None,
            config: json!({}),
            environments: envs,
            env_vars: String::new(),
            status: "active".into(),
            created_by_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn env(id: &str, branch: &str) -> Value {
        json!({ "id": id, "name": id, "slug": id, "branch": branch, "status": "active" })
    }

    #[test]
    fn env_for_branch_exact_prod() {
        let p = project_with_envs(json!([env("prod", "main"), env("staging", "dev*")]));
        assert_eq!(p.environment_for_branch("main").unwrap().id, "prod");
    }

    #[test]
    fn env_for_branch_wildcards() {
        let p = project_with_envs(json!([
            env("prod", "main"),
            env("staging", "dev*"),
            env("review", "*-pr"),
        ]));
        assert_eq!(p.environment_for_branch("dev-feat").unwrap().id, "staging");
        assert_eq!(p.environment_for_branch("x-pr").unwrap().id, "review");
        assert!(p.environment_for_branch("random").is_none());
    }

    #[test]
    fn env_vars_flatten_env_overrides_global() {
        let crypto = Crypto::new("k").unwrap();
        let mut p = project_with_envs(json!([]));
        p.set_env_vars(
            &crypto,
            &[
                EnvVar {
                    key: "A".into(),
                    value: "global".into(),
                    environment: None,
                },
                EnvVar {
                    key: "A".into(),
                    value: "staging".into(),
                    environment: Some("staging".into()),
                },
                EnvVar {
                    key: "B".into(),
                    value: "only-staging".into(),
                    environment: Some("staging".into()),
                },
            ],
        )
        .unwrap();
        let vars = p.env_vars_for(&crypto, "staging").unwrap();
        let map: std::collections::HashMap<_, _> = vars
            .iter()
            .map(|v| (v.key.as_str(), v.value.as_str()))
            .collect();
        assert_eq!(map["A"], "staging");
        assert_eq!(map["B"], "only-staging");
        let prod = p.env_vars_for(&crypto, "prod").unwrap();
        assert_eq!(prod.len(), 1);
        assert_eq!(prod[0].value, "global");
    }

    fn deployment(
        container: Option<&str>,
        observed: Option<&str>,
        exit: Option<i32>,
    ) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: "d0123456789abcdef".into(),
            project_id: "p1".into(),
            repo_provider: "github".into(),
            repo_id: 1,
            repo_full_name: "o/r".into(),
            repo_base_url: "https://github.com".into(),
            environment_id: "prod".into(),
            branch: "main".into(),
            commit_sha: "abc".into(),
            commit_meta: json!({}),
            config: json!({}),
            image: None,
            env_vars: String::new(),
            job_id: None,
            error: None,
            container_id: None,
            container_status: container.map(String::from),
            observed_status: observed.map(String::from),
            observed_exit_code: exit,
            observed_at: None,
            observed_reason: None,
            observed_last_seen_at: None,
            observed_missing_count: 0,
            status: "completed".into(),
            conclusion: Some("succeeded".into()),
            trigger: "user".into(),
            created_by_user_id: None,
            created_at: now,
            concluded_at: None,
        }
    }

    #[test]
    fn computed_status_matrix() {
        assert_eq!(
            deployment(Some("running"), Some("running"), None).computed_status(),
            "running"
        );
        assert_eq!(
            deployment(Some("running"), Some("not_found"), None).computed_status(),
            "missing"
        );
        assert_eq!(
            deployment(Some("running"), Some("exited"), Some(0)).computed_status(),
            "stopped"
        );
        assert_eq!(
            deployment(Some("running"), Some("exited"), Some(1)).computed_status(),
            "crashed"
        );
        assert_eq!(
            deployment(Some("stopped"), Some("running"), None).computed_status(),
            "orphaned"
        );
    }

    #[test]
    fn deployment_port_defaults_and_overrides() {
        let mut d = deployment(None, None, None);
        assert_eq!(d.deployment_port(), 8000);
        d.config = json!({ "port": 3000 });
        assert_eq!(d.deployment_port(), 3000);
        d.config = json!({ "port": "3001" });
        assert_eq!(d.deployment_port(), 3001);
    }

    #[test]
    fn deployment_slug_truncates_id() {
        let d = deployment(None, None, None);
        assert_eq!(d.slug("proj"), "proj-id-d012345");
    }
}
