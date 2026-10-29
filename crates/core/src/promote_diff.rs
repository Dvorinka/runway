//! What a promotion would change. Values never leave this module:
//! callers get key names only. Redirect rules are project-scoped, so
//! that list is empty — a promotion does not edit them. Custom domains
//! follow the environment-id alias set at deploy time, not the alias
//! `promote` moves, so they are reported as unchanged.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::crypto::Crypto;
use crate::deploy;
use crate::models::{Deployment, EnvVar, Project};
use crate::Settings;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl KeyDiff {
    fn empty() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PromotionDiff {
    pub environment_id: String,
    pub from_deployment_id: Option<String>,
    pub to_deployment_id: String,
    pub already_current: bool,
    pub promotable: bool,
    /// Hostname of the environment alias this promotion would move.
    pub alias: String,
    pub env: KeyDiff,
    pub config: KeyDiff,
    /// Active `route` domains bound to the target environment.
    /// Promote does not retarget them.
    pub domains_unchanged: Vec<String>,
    pub redirects: KeyDiff,
}

fn diff_keys<T: PartialEq>(current: &BTreeMap<String, T>, next: &BTreeMap<String, T>) -> KeyDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (key, value) in next {
        match current.get(key) {
            None => added.push(key.clone()),
            Some(old) if old != value => changed.push(key.clone()),
            _ => {}
        }
    }
    for key in current.keys() {
        if !next.contains_key(key) {
            removed.push(key.clone());
        }
    }
    KeyDiff {
        added,
        removed,
        changed,
    }
}

fn env_map(vars: &[EnvVar]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for var in vars {
        if !var.key.is_empty() {
            map.insert(var.key.clone(), var.value.clone());
        }
    }
    map
}

fn config_map(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn diff_env(current: &[EnvVar], next: &[EnvVar]) -> KeyDiff {
    diff_keys(&env_map(current), &env_map(next))
}

fn diff_config(current: &Value, next: &Value) -> KeyDiff {
    diff_keys(&config_map(current), &config_map(next))
}

/// Preview of `deploy::promote`. Does not write.
pub async fn preview(
    db: &PgPool,
    crypto: &Crypto,
    settings: &Settings,
    project: &Project,
    deployment: &Deployment,
    environment_id: &str,
) -> Result<PromotionDiff> {
    let subdomain = deploy::environment_subdomain(project, environment_id)?;
    let alias_row = deploy::environment_alias(db, project, environment_id).await?;
    let already_current = alias_row
        .as_ref()
        .is_some_and(|alias| alias.deployment_id == deployment.id);

    let (from_deployment_id, current_env, current_config) = match &alias_row {
        Some(alias) if alias.deployment_id == deployment.id => (
            Some(deployment.id.clone()),
            deployment.env_vars(crypto)?,
            deployment.config.clone(),
        ),
        Some(alias) => match deploy::get(db, &alias.deployment_id).await? {
            Some(prev) => (
                Some(prev.id.clone()),
                prev.env_vars(crypto)?,
                prev.config.clone(),
            ),
            None => (Some(alias.deployment_id.clone()), Vec::new(), Value::Null),
        },
        None => (None, Vec::new(), Value::Null),
    };

    let domains: Vec<(String,)> = sqlx::query_as(
        "SELECT hostname FROM domain
         WHERE project_id = $1 AND status = 'active' AND type = 'route'
           AND environment_id = $2
         ORDER BY hostname",
    )
    .bind(&project.id)
    .bind(environment_id)
    .fetch_all(db)
    .await
    .map_err(Error::from)?;

    let promotable =
        deployment.status == "completed" && deployment.conclusion.as_deref() == Some("succeeded");

    Ok(PromotionDiff {
        environment_id: environment_id.to_string(),
        from_deployment_id,
        to_deployment_id: deployment.id.clone(),
        already_current,
        promotable,
        alias: format!("{subdomain}.{}", settings.deploy_domain),
        env: diff_env(&current_env, &deployment.env_vars(crypto)?),
        config: diff_config(&current_config, &deployment.config),
        domains_unchanged: domains.into_iter().map(|(host,)| host).collect(),
        redirects: KeyDiff::empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn var(key: &str, value: &str) -> EnvVar {
        EnvVar {
            key: key.into(),
            value: value.into(),
            environment: None,
        }
    }

    #[test]
    fn env_diff_lists_keys_and_drops_values() {
        let current = vec![var("KEEP", "a"), var("GONE", "secret"), var("TOKEN", "old")];
        let next = vec![
            var("KEEP", "a"),
            var("TOKEN", "new"),
            var("FRESH", "also-secret"),
        ];
        let diff = diff_env(&current, &next);
        assert_eq!(diff.added, vec!["FRESH".to_string()]);
        assert_eq!(diff.removed, vec!["GONE".to_string()]);
        assert_eq!(diff.changed, vec!["TOKEN".to_string()]);
        let rendered = serde_json::to_string(&diff).unwrap();
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("\"old\""));
        assert!(!rendered.contains("\"new\""));
    }

    #[test]
    fn later_duplicate_key_wins() {
        let current = vec![var("A", "1"), var("A", "2")];
        let next = vec![var("A", "2")];
        assert!(diff_env(&current, &next).is_empty());
    }

    #[test]
    fn config_diff_is_top_level_and_sorted() {
        let current = json!({"build_command": "npm run build", "port": 3000, "zeta": 1});
        let next = json!({"build_command": "pnpm build", "output_directory": "dist", "zeta": 1});
        let diff = diff_config(&current, &next);
        assert_eq!(diff.added, vec!["output_directory".to_string()]);
        assert_eq!(diff.removed, vec!["port".to_string()]);
        assert_eq!(diff.changed, vec!["build_command".to_string()]);
    }

    #[test]
    fn nested_config_equality_ignores_key_order() {
        let current = json!({"firewall": {"ips": ["1.1.1.1"], "limit": 10}});
        let next = json!({"firewall": {"limit": 10, "ips": ["1.1.1.1"]}});
        assert!(diff_config(&current, &next).is_empty());
    }

    #[test]
    fn missing_side_is_all_added() {
        let next = vec![var("ONLY", "x")];
        assert_eq!(diff_env(&[], &next).added, vec!["ONLY".to_string()]);
    }
}
