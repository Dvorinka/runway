//! Deployment service — port of services/deployment.py.
//!
//! Covers creation, status transitions (with event emission), alias
//! management, env-var flattening, cancel/skip/rollback, and job enqueue.

use std::collections::HashMap;

use bollard::Docker;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::config::Settings;
use crate::crypto::Crypto;
use crate::docker as dkr;
use crate::error::{Error, Result};
use crate::events::EventBus;
use crate::models::{Alias, Deployment, Project};
use crate::slugify::{branch_slug, token_hex};
use crate::traefik;

pub const EDGE_NETWORK_PREFIX: &str = "runway_edge_";
pub const WORKSPACE_NETWORK_PREFIX: &str = "runway_ws_";

pub fn edge_network_name(deployment_id: &str) -> String {
    format!("{EDGE_NETWORK_PREFIX}{deployment_id}")
}

pub fn workspace_network_name(team_id: &str) -> String {
    format!("{WORKSPACE_NETWORK_PREFIX}{team_id}")
}

/// Commit info passed at creation — normalized shape used by
/// webhooks and manual deploys.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: Option<String>,
}

impl CommitInfo {
    pub fn to_meta(&self) -> Value {
        json!({
            "sha": self.sha,
            "author": self.author,
            "message": self.message,
            "timestamp": self.timestamp,
        })
    }
}

/// Resolve `branch`'s head commit through the project's provider —
/// GitHub installation tokens for github/GHE, stored connection
/// tokens for gitea/gitlab. Shared by API routes and the cron tick.
pub async fn resolve_commit(
    db: &PgPool,
    crypto: &Crypto,
    github: Option<&crate::github::GithubService>,
    project: &Project,
    branch: &str,
) -> Result<CommitInfo> {
    match project.repo_provider.as_str() {
        "github" | "github_enterprise" => {
            let gh =
                github.ok_or_else(|| Error::Validation("GitHub App is not configured".into()))?;
            let installation_id = project
                .github_installation_id
                .ok_or_else(|| Error::Validation("project has no GitHub installation".into()))?;
            let token = gh.installation_token(db, crypto, installation_id).await?;
            let commit = gh
                .latest_commit(&token, &project.repo_full_name, branch)
                .await?;
            Ok(CommitInfo {
                sha: commit["sha"].as_str().unwrap_or_default().to_string(),
                message: commit["commit"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                author: commit["commit"]["author"]["name"]
                    .as_str()
                    .or_else(|| commit["author"]["login"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                timestamp: commit["commit"]["author"]["date"]
                    .as_str()
                    .map(String::from),
            })
        }
        p @ ("gitea" | "gitlab" | "bitbucket") => {
            let conn_id = match p {
                "gitea" => project.gitea_connection_id,
                "gitlab" => project.gitlab_connection_id,
                _ => project.bitbucket_connection_id,
            }
            .ok_or_else(|| Error::Validation(format!("project has no {p} connection")))?;
            let conn = crate::git_providers::connection(db, crypto, p, conn_id)
                .await?
                .ok_or_else(|| Error::Validation(format!("{p} connection not found")))?;
            let (owner, repo) = project
                .repo_full_name
                .rsplit_once('/')
                .ok_or_else(|| Error::Validation("invalid repo_full_name".into()))?;
            let commit = crate::git_providers::Client::new(p, conn)
                .latest_commit(owner, repo, branch)
                .await?
                .ok_or_else(|| {
                    Error::Validation(format!("branch '{branch}' not found on remote"))
                })?;
            Ok(CommitInfo {
                sha: commit.sha,
                message: commit.message,
                author: commit.author,
                timestamp: commit.timestamp,
            })
        }
        other => Err(Error::Validation(format!(
            "repo provider '{other}' not supported"
        ))),
    }
}

pub async fn enqueue(db: &PgPool, kind: &str, payload: Value, defer_seconds: i64) -> Result<i64> {
    let run_at = Utc::now() + Duration::seconds(defer_seconds);
    let (id,): (i64,) =
        sqlx::query_as("INSERT INTO job (kind, payload, run_at) VALUES ($1, $2, $3) RETURNING id")
            .bind(kind)
            .bind(payload)
            .bind(run_at)
            .fetch_one(db)
            .await?;
    Ok(id)
}

/// Create a deployment row + enqueue `start_deployment`.
/// Port of DeploymentService.create (branch→env mapping, config snapshot).
#[allow(clippy::too_many_arguments)]
pub async fn create(
    db: &PgPool,
    bus: &EventBus,
    crypto: &Crypto,
    project: &Project,
    branch: &str,
    commit: &CommitInfo,
    trigger: &str,
    user_id: Option<i64>,
    extra_config: Option<Value>,
) -> Result<Deployment> {
    let environment = project
        .environment_for_branch(branch)
        .ok_or_else(|| Error::Validation(format!("no environment matches branch '{branch}'")))?;

    let env_vars = project.env_vars_for(crypto, &environment.slug)?;
    let env_vars_enc = crypto.encrypt(&serde_json::to_string(&env_vars)?)?;

    let mut config = project.config.clone();
    if let Some(extra) = extra_config {
        if let (Some(base), Some(over)) = (config.as_object_mut(), extra.as_object()) {
            for (k, v) in over {
                base.insert(k.clone(), v.clone());
            }
        }
    }

    let id = token_hex(16);
    sqlx::query(
        "INSERT INTO deployment (
            id, project_id, repo_provider, repo_id, repo_full_name, repo_base_url,
            environment_id, branch, commit_sha, commit_meta, config, env_vars,
            status, trigger, created_by_user_id
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'prepare',$13,$14)",
    )
    .bind(&id)
    .bind(&project.id)
    .bind(&project.repo_provider)
    .bind(project.repo_id.unwrap_or(0))
    .bind(&project.repo_full_name)
    .bind(&project.repo_base_url)
    .bind(&environment.id)
    .bind(branch)
    .bind(&commit.sha)
    .bind(commit.to_meta())
    .bind(&config)
    .bind(&env_vars_enc)
    .bind(trigger)
    .bind(user_id)
    .execute(db)
    .await?;

    let job_id = enqueue(db, "start_deployment", json!({ "deployment_id": id }), 0).await?;
    sqlx::query("UPDATE deployment SET job_id = $1 WHERE id = $2")
        .bind(job_id.to_string())
        .bind(&id)
        .execute(db)
        .await?;

    publish_update(bus, &project.id, &id, "prepare");

    get(db, &id)
        .await?
        .ok_or_else(|| Error::NotFound("deployment".into()))
}

pub async fn get(db: &PgPool, id: &str) -> Result<Option<Deployment>> {
    Ok(
        sqlx::query_as::<_, Deployment>("SELECT * FROM deployment WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?,
    )
}

pub async fn get_project(db: &PgPool, id: &str) -> Result<Option<Project>> {
    Ok(
        sqlx::query_as::<_, Project>("SELECT * FROM project WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?,
    )
}

/// Patch status fields + emit a project-scoped event.
/// Port of DeploymentService.update_status.
#[allow(clippy::too_many_arguments)]
pub async fn update_status(
    db: &PgPool,
    bus: &EventBus,
    deployment_id: &str,
    status: Option<&str>,
    conclusion: Option<&str>,
    error: Option<Value>,
    container_status: Option<&str>,
) -> Result<()> {
    if status.is_none() && conclusion.is_none() && error.is_none() && container_status.is_none() {
        return Ok(());
    }
    let mut tx = db.begin().await?;
    if let Some(s) = status {
        sqlx::query("UPDATE deployment SET status = $1 WHERE id = $2")
            .bind(s)
            .bind(deployment_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(c) = conclusion {
        sqlx::query("UPDATE deployment SET conclusion = $1, concluded_at = now() WHERE id = $2")
            .bind(c)
            .bind(deployment_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(e) = error {
        sqlx::query("UPDATE deployment SET error = $1 WHERE id = $2")
            .bind(e)
            .bind(deployment_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(cs) = container_status {
        sqlx::query("UPDATE deployment SET container_status = $1 WHERE id = $2")
            .bind(cs)
            .bind(deployment_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    if let Some(dep) = get(db, deployment_id).await? {
        let value = conclusion.or(status).unwrap_or_default();
        bus.publish(
            &format!("project:{}:updates", dep.project_id),
            &json!({
                "event_type": "deployment_status_update",
                "project_id": dep.project_id,
                "deployment_id": dep.id,
                "deployment_status": value,
                "timestamp": Utc::now().to_rfc3339(),
            }),
        );
    }
    Ok(())
}

fn publish_update(bus: &EventBus, project_id: &str, deployment_id: &str, status: &str) {
    bus.publish(
        &format!("project:{project_id}:updates"),
        &json!({
            "event_type": "deployment_status_update",
            "project_id": project_id,
            "deployment_id": deployment_id,
            "deployment_status": status,
            "timestamp": Utc::now().to_rfc3339(),
        }),
    );
}

/// Subdomain map for a deployment — port of get_alias_domains.
pub fn alias_domains(
    deployment: &Deployment,
    project: &Project,
    settings: &Settings,
) -> HashMap<String, String> {
    let slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let mut v = HashMap::new();

    if !deployment.branch.is_empty() {
        let sanitized = branch_slug(&deployment.branch);
        if !sanitized.is_empty() {
            let sub = format!("{slug}-branch-{sanitized}");
            v.insert("branch_subdomain".into(), sub.clone());
            v.insert(
                "branch_domain".into(),
                format!("{sub}.{}", settings.deploy_domain),
            );
            v.insert(
                "branch_url".into(),
                format!(
                    "{}://{}.{}",
                    settings.url_scheme, sub, settings.deploy_domain
                ),
            );
        }
    }

    if deployment.environment_id == "prod" {
        v.insert("environment_subdomain".into(), slug.clone());
    } else if let Some(env) = project.environment_by_id(&deployment.environment_id) {
        v.insert(
            "environment_subdomain".into(),
            format!("{slug}-env-{}", env.slug),
        );
    }
    if let Some(sub) = v.get("environment_subdomain").cloned() {
        v.insert(
            "environment_domain".into(),
            format!("{sub}.{}", settings.deploy_domain),
        );
        v.insert(
            "environment_url".into(),
            format!(
                "{}://{}.{}",
                settings.url_scheme, sub, settings.deploy_domain
            ),
        );
    }

    let id_sub = format!("{slug}-env-id-{}", deployment.environment_id);
    v.insert("environment_id_subdomain".into(), id_sub.clone());
    v.insert(
        "environment_id_domain".into(),
        format!("{id_sub}.{}", settings.deploy_domain),
    );
    v.insert(
        "environment_id_url".into(),
        format!(
            "{}://{}.{}",
            settings.url_scheme, id_sub, settings.deploy_domain
        ),
    );

    v
}

/// Runner env vars for a deployment — port of get_runtime_env_vars
/// with the DEVPUSH_ prefix renamed RUNWAY_.
pub fn runtime_env_vars(
    deployment: &Deployment,
    project: &Project,
    settings: &Settings,
    crypto: &Crypto,
) -> Result<Vec<(String, String)>> {
    let mut vars: HashMap<String, String> = deployment
        .env_vars(crypto)?
        .into_iter()
        .map(|v| (v.key, v.value))
        .collect();

    let env_slug = project
        .environment_by_id(&deployment.environment_id)
        .map(|e| e.slug)
        .unwrap_or_else(|| deployment.environment_id.clone());
    let aliases = alias_domains(deployment, project, settings);

    let mut runtime = HashMap::from([
        ("RUNWAY".into(), "true".into()),
        (
            "RUNWAY_URL".into(),
            deployment.url(&project.slug.clone().unwrap_or_default(), settings),
        ),
        (
            "RUNWAY_DOMAIN".into(),
            deployment.hostname(&project.slug.clone().unwrap_or_default(), settings),
        ),
        ("RUNWAY_TEAM_ID".into(), project.team_id.clone()),
        ("RUNWAY_PROJECT_ID".into(), project.id.clone()),
        ("RUNWAY_ENVIRONMENT".into(), env_slug),
        ("RUNWAY_DEPLOYMENT_ID".into(), deployment.id.clone()),
        (
            "RUNWAY_DEPLOYMENT_CREATED_AT".into(),
            deployment.created_at.to_rfc3339(),
        ),
        (
            "RUNWAY_GIT_PROVIDER".into(),
            deployment.repo_provider.clone(),
        ),
        (
            "RUNWAY_GIT_BASE_URL".into(),
            deployment.repo_base_url.clone(),
        ),
        ("RUNWAY_GIT_REPO".into(), deployment.repo_full_name.clone()),
        ("RUNWAY_GIT_REF".into(), deployment.branch.clone()),
        (
            "RUNWAY_GIT_COMMIT_SHA".into(),
            deployment.commit_sha.clone(),
        ),
        ("PUID".into(), settings.service_uid.to_string()),
        ("PGID".into(), settings.service_gid.to_string()),
        ("PORT".into(), deployment.deployment_port().to_string()),
    ]);

    if let Some(ip) = &settings.server_ip {
        runtime.insert("RUNWAY_IP".into(), ip.clone());
    }
    if let Some(d) = aliases.get("environment_domain") {
        runtime.insert("RUNWAY_DOMAIN_ENVIRONMENT".into(), d.clone());
    }
    if let Some(u) = aliases.get("environment_url") {
        runtime.insert("RUNWAY_URL_ENVIRONMENT".into(), u.clone());
    }
    if let Some(d) = aliases.get("branch_domain") {
        runtime.insert("RUNWAY_DOMAIN_BRANCH".into(), d.clone());
    }
    if let Some(u) = aliases.get("branch_url") {
        runtime.insert("RUNWAY_URL_BRANCH".into(), u.clone());
    }

    if let Some(author) = deployment
        .commit_meta
        .get("author")
        .and_then(|v| v.as_str())
    {
        runtime.insert("RUNWAY_GIT_COMMIT_AUTHOR".into(), author.to_string());
    }
    if let Some(msg) = deployment
        .commit_meta
        .get("message")
        .and_then(|v| v.as_str())
    {
        runtime.insert("RUNWAY_GIT_COMMIT_MESSAGE".into(), msg.to_string());
    }
    if let Some((owner, repo)) = deployment.repo_full_name.split_once('/') {
        runtime.insert("RUNWAY_GIT_REPO_OWNER".into(), owner.to_string());
        runtime.insert("RUNWAY_GIT_REPO_NAME".into(), repo.to_string());
    }

    for (k, v) in runtime {
        if !v.is_empty() {
            vars.entry(k).or_insert(v);
        }
    }
    Ok(vars.into_iter().collect())
}

/// Alias update_or_create — tracks previous_deployment for prod rollback.
pub async fn alias_update_or_create(
    db: &PgPool,
    subdomain: &str,
    deployment_id: &str,
    r#type: &str,
    value: Option<&str>,
    environment_id: &str,
) -> Result<()> {
    let existing: Option<Alias> = sqlx::query_as("SELECT * FROM alias WHERE subdomain = $1")
        .bind(subdomain)
        .fetch_optional(db)
        .await?;

    if let Some(alias) = existing {
        if alias.deployment_id == deployment_id {
            return Ok(());
        }
        let previous = if r#type == "environment" && environment_id == "prod" {
            Some(alias.deployment_id)
        } else {
            None
        };
        sqlx::query(
            "UPDATE alias SET deployment_id = $1, previous_deployment_id = $2, updated_at = now()
             WHERE subdomain = $3",
        )
        .bind(deployment_id)
        .bind(previous)
        .bind(subdomain)
        .execute(db)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO alias (subdomain, deployment_id, type, value) VALUES ($1,$2,$3,$4)",
        )
        .bind(subdomain)
        .bind(deployment_id)
        .bind(r#type)
        .bind(value)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Create branch/environment/environment-id aliases for a deployment.
/// Port of setup_aliases.
pub async fn setup_aliases(
    db: &PgPool,
    deployment: &Deployment,
    project: &Project,
    settings: &Settings,
) -> Result<()> {
    let domains = alias_domains(deployment, project, settings);

    if let Some(sub) = domains.get("branch_subdomain") {
        if let Err(e) = alias_update_or_create(
            db,
            sub,
            &deployment.id,
            "branch",
            Some(&deployment.branch),
            &deployment.environment_id,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to create branch alias");
        }
    }
    if let Some(sub) = domains.get("environment_subdomain") {
        if let Err(e) = alias_update_or_create(
            db,
            sub,
            &deployment.id,
            "environment",
            Some(&deployment.environment_id),
            &deployment.environment_id,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to create environment alias");
        }
    }
    if let Some(sub) = domains.get("environment_id_subdomain") {
        if let Err(e) = alias_update_or_create(
            db,
            sub,
            &deployment.id,
            "environment_id",
            Some(&deployment.environment_id),
            &deployment.environment_id,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to create environment-id alias");
        }
    }
    Ok(())
}

/// Cancel a deployment — port of DeploymentService.cancel.
/// Sets conclusion, stops the container, schedules deletion.
pub async fn cancel(
    db: &PgPool,
    bus: &EventBus,
    settings: &Settings,
    docker: &Docker,
    deployment: &Deployment,
) -> Result<()> {
    if deployment.conclusion.is_some() {
        return Err(Error::Validation("deployment already concluded".into()));
    }
    if !["prepare", "deploy"].contains(&deployment.status.as_str()) {
        return Err(Error::Validation(
            "deployment cannot be canceled at this stage".into(),
        ));
    }

    update_status(
        db,
        bus,
        &deployment.id,
        Some("completed"),
        Some("canceled"),
        None,
        None,
    )
    .await?;

    if let Some(container_id) = &deployment.container_id {
        let _ = docker.stop_container(container_id, None).await;
        update_status(db, bus, &deployment.id, None, None, None, Some("stopped")).await?;
        enqueue(
            db,
            "delete_container",
            json!({ "deployment_id": deployment.id }),
            settings.container_delete_grace_seconds as i64,
        )
        .await?;
        let _ = dkr::remove_network_if_empty(docker, &edge_network_name(&deployment.id)).await;
    }
    Ok(())
}

/// Skip — mark a pending deployment as skipped without touching containers.
pub async fn skip(db: &PgPool, bus: &EventBus, deployment: &Deployment) -> Result<()> {
    if deployment.conclusion.is_some() {
        return Err(Error::Validation("deployment already concluded".into()));
    }
    if ["finalize", "fail", "completed"].contains(&deployment.status.as_str()) {
        return Err(Error::Validation("deployment already finalizing".into()));
    }
    update_status(
        db,
        bus,
        &deployment.id,
        Some("completed"),
        Some("skipped"),
        None,
        None,
    )
    .await
}

/// Rollback an environment to its previous deployment — port of rollback.
pub async fn rollback(
    db: &PgPool,
    bus: &EventBus,
    settings: &Settings,
    project: &Project,
    environment_id: &str,
) -> Result<Alias> {
    let slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let subdomain = if environment_id == "prod" {
        slug.clone()
    } else {
        let env = project
            .environment_by_id(environment_id)
            .ok_or_else(|| Error::NotFound("environment".into()))?;
        format!("{slug}-env-{}", env.slug)
    };

    let alias: Option<Alias> = sqlx::query_as("SELECT * FROM alias WHERE subdomain = $1")
        .bind(&subdomain)
        .fetch_optional(db)
        .await?;
    let Some(alias) = alias else {
        return Err(Error::Validation(
            "no previous deployment to roll back to".into(),
        ));
    };
    let Some(prev) = alias.previous_deployment_id.clone() else {
        return Err(Error::Validation(
            "no previous deployment to roll back to".into(),
        ));
    };

    sqlx::query(
        "UPDATE alias SET deployment_id = $1, previous_deployment_id = $2, updated_at = now()
         WHERE id = $3",
    )
    .bind(&prev)
    .bind(&alias.deployment_id)
    .bind(alias.id)
    .execute(db)
    .await?;

    traefik::update_project_config(db, project, settings, &[]).await?;

    bus.publish(
        &format!("project:{}:updates", project.id),
        &json!({
            "event_type": "deployment_rollback",
            "environment_id": environment_id,
            "deployment_id": prev,
            "previous_deployment_id": alias.deployment_id,
            "timestamp": Utc::now().to_rfc3339(),
        }),
    );

    sqlx::query_as("SELECT * FROM alias WHERE id = $1")
        .bind(alias.id)
        .fetch_one(db)
        .await
        .map_err(Error::from)
}

/// Remove a deployment's edge network (post stop/delete).
pub async fn cleanup_edge_network(docker: &Docker, deployment_id: &str) -> Result<()> {
    let edge = edge_network_name(deployment_id);
    let traefik_id = dkr::service_container_id(docker, "traefik").await;
    dkr::disconnect_from_network(docker, traefik_id.as_deref(), Some(&edge)).await?;
    let _ = dkr::remove_network_if_empty(docker, &edge).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_settings;

    fn deployment(env_id: &str, branch: &str) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: "d0123456789abcdef".into(),
            project_id: "p1".into(),
            repo_provider: "github".into(),
            repo_id: 1,
            repo_full_name: "o/r".into(),
            repo_base_url: "https://github.com".into(),
            environment_id: env_id.into(),
            branch: branch.into(),
            commit_sha: "abc".into(),
            commit_meta: json!({}),
            config: json!({}),
            image: None,
            env_vars: String::new(),
            job_id: None,
            error: None,
            container_id: None,
            container_status: None,
            observed_status: None,
            observed_exit_code: None,
            observed_at: None,
            observed_reason: None,
            observed_last_seen_at: None,
            observed_missing_count: 0,
            status: "deploy".into(),
            conclusion: None,
            trigger: "user".into(),
            remote_node_id: None,
            remote_port: None,
            created_by_user_id: None,
            created_at: now,
            concluded_at: None,
        }
    }

    fn project() -> Project {
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
            bitbucket_connection_id: None,
            remote_node_id: None,
            has_avatar: false,
            config: json!({}),
            environments: json!([
                { "id": "prod", "name": "Production", "slug": "production",
                  "branch": "main", "status": "active" },
                { "id": "stg", "name": "Staging", "slug": "staging",
                  "branch": "dev*", "status": "active" },
            ]),
            env_vars: String::new(),
            status: "active".into(),
            created_by_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn alias_domains_prod() {
        let s = test_settings();
        let d = deployment("prod", "main");
        let m = alias_domains(&d, &project(), &s);
        assert_eq!(m["branch_subdomain"], "proj-branch-main");
        assert_eq!(m["environment_subdomain"], "proj");
        assert_eq!(m["environment_domain"], "proj.deploy.test");
        assert_eq!(m["environment_id_subdomain"], "proj-env-id-prod");
        assert_eq!(m["environment_url"], "https://proj.deploy.test");
    }

    #[test]
    fn alias_domains_nonprod() {
        let s = test_settings();
        let d = deployment("stg", "dev-feat");
        let m = alias_domains(&d, &project(), &s);
        assert_eq!(m["environment_subdomain"], "proj-env-staging");
        assert_eq!(m["environment_id_subdomain"], "proj-env-id-stg");
        assert_eq!(m["branch_domain"], "proj-branch-dev-feat.deploy.test");
    }
}
