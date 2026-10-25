//! Outbound deployment webhooks — port of devpush services/webhook.py.
//!
//! Fan-out on lifecycle events to (in order): the project's legacy
//! `config.webhook_url`, `project_webhook` rows, `team_webhook` rows.
//! Delivery is a POST with `X-Runway-Event`, `X-Runway-Delivery`, and
//! `X-Runway-Signature: sha256=<hmac>` when a secret is configured.

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::config::Settings;
use crate::crypto::Crypto;
use crate::error::Result;
use crate::models::{Deployment, Project, ProjectWebhook, TeamWebhook};

pub const WEBHOOK_EVENTS: [&str; 6] = [
    "started",
    "succeeded",
    "failed",
    "canceled",
    "skipped",
    "crashed",
];

/// Port of _build_deployment_payload.
fn build_payload(project: &Project, dep: &Deployment, event: &str, settings: &Settings) -> Value {
    let environment = project
        .environments()
        .into_iter()
        .find(|e| e.id == dep.environment_id);
    let meta = &dep.commit_meta;
    json!({
        "event": format!("deployment.{event}"),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "project": {
            "id": project.id,
            "name": project.name,
            "slug": project.slug,
            "repo_full_name": project.repo_full_name,
        },
        "deployment": {
            "id": dep.id,
            "status": dep.status,
            "conclusion": dep.conclusion,
            "branch": dep.branch,
            "commit_sha": dep.commit_sha,
            "commit_message": meta.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            "commit_author": meta.get("author").and_then(|v| v.as_str()).unwrap_or(""),
            "environment": {
                "id": dep.environment_id,
                "name": environment.as_ref().map(|e| e.name.clone()),
                "slug": environment.as_ref().map(|e| e.slug.clone()),
            },
            "trigger": dep.trigger,
            // dep.url() already carries the scheme — wrapping it again
            // produced https://https://… (devpush builds scheme://slug.domain).
            "url": dep.url(project.slug.as_deref().unwrap_or(&project.id), settings),
            "created_at": dep.created_at.to_rfc3339(),
            "concluded_at": dep.concluded_at.map(|t| t.to_rfc3339()),
        },
    })
}

/// Port of _deliver_webhook. Best-effort: logs and returns on failure.
async fn deliver(url: &str, payload: &Value, event: &str, delivery_id: &str, secret: Option<&str>) {
    let body = payload.to_string();
    let mut req = reqwest::Client::new()
        .post(url)
        .timeout(std::time::Duration::from_secs(10))
        .header("content-type", "application/json")
        .header("user-agent", "runway-webhook/1.0")
        .header("x-runway-event", format!("deployment.{event}"))
        .header("x-runway-delivery", delivery_id)
        .body(body.clone());
    if let Some(secret) = secret {
        let sig = crate::crypto::hmac_sha256_hex(secret.as_bytes(), body.as_bytes());
        req = req.header("x-runway-signature", format!("sha256={sig}"));
    }
    match req.send().await {
        Ok(resp) if resp.status().is_client_error() || resp.status().is_server_error() => {
            tracing::warn!(url, status = %resp.status(), "webhook delivery failed");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(url, error = %e, "webhook delivery failed"),
    }
}

fn wants(events: &Value, event: &str) -> bool {
    events
        .as_array()
        .filter(|a| !a.is_empty())
        .map(|a| a.iter().any(|v| v.as_str() == Some(event)))
        .unwrap_or_else(|| WEBHOOK_EVENTS.contains(&event))
}

/// Port of send_deployment_webhook + send_team/project_webhooks.
/// Never fails the caller — delivery errors are logged only.
pub async fn send_deployment_webhooks(
    db: &PgPool,
    crypto: &Crypto,
    settings: &Settings,
    project: &Project,
    dep: &Deployment,
    event: &str,
) {
    let payload = build_payload(project, dep, event, settings);
    let decrypt = |ct: &Option<String>| -> Option<String> {
        ct.as_deref().and_then(|s| crypto.decrypt(s).ok())
    };

    // Legacy project-config webhook (config.webhook_url) — devpush parity.
    if let Some(url) = project.config.get("webhook_url").and_then(|v| v.as_str()) {
        let events = project
            .config
            .get("webhook_events")
            .cloned()
            .unwrap_or(Value::Null);
        if wants(&events, event) {
            let secret = project
                .config
                .get("webhook_secret")
                .and_then(|v| v.as_str());
            deliver(url, &payload, event, &dep.id, secret).await;
        }
    }

    let project_hooks: Vec<ProjectWebhook> =
        sqlx::query_as("SELECT * FROM project_webhook WHERE project_id = $1 AND status = 'active'")
            .bind(&project.id)
            .fetch_all(db)
            .await
            .unwrap_or_default();
    for hook in project_hooks {
        if !wants(&hook.events, event) {
            continue;
        }
        let delivery = format!("{}-{}", dep.id, &hook.id[..8.min(hook.id.len())]);
        deliver(
            &hook.url,
            &payload,
            event,
            &delivery,
            decrypt(&hook.secret).as_deref(),
        )
        .await;
    }

    let team_hooks: Vec<TeamWebhook> =
        sqlx::query_as("SELECT * FROM team_webhook WHERE team_id = $1 AND status = 'active'")
            .bind(&project.team_id)
            .fetch_all(db)
            .await
            .unwrap_or_default();
    for hook in team_hooks {
        if !hook.applies_to_project(&project.id) || !wants(&hook.events, event) {
            continue;
        }
        let delivery = format!("{}-{}", dep.id, &hook.id[..8.min(hook.id.len())]);
        deliver(
            &hook.url,
            &payload,
            event,
            &delivery,
            decrypt(&hook.secret).as_deref(),
        )
        .await;
    }
}

/// For handlers that only know ids (cancel/skip paths).
pub async fn send_for_ids(
    db: &PgPool,
    crypto: &Crypto,
    settings: &Settings,
    project_id: &str,
    deployment_id: &str,
    event: &str,
) -> Result<()> {
    let Some(project) = crate::deploy::get_project(db, project_id).await? else {
        return Ok(());
    };
    let Some(dep) = crate::deploy::get(db, deployment_id).await? else {
        return Ok(());
    };
    send_deployment_webhooks(db, crypto, settings, &project, &dep, event).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_settings;

    fn project() -> Project {
        let now = chrono::Utc::now();
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
            ]),
            env_vars: String::new(),
            status: "active".into(),
            created_by_user_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn payload_url_is_single_schemed_immutable_url() {
        let s = test_settings();
        let p = project();
        let mut d = Deployment {
            id: "d0123456789abcdef".into(),
            project_id: "p1".into(),
            repo_provider: "github".into(),
            repo_id: 1,
            repo_full_name: "o/r".into(),
            repo_base_url: "https://github.com".into(),
            environment_id: "prod".into(),
            branch: "main".into(),
            commit_sha: "abc".into(),
            commit_meta: json!({"message": "m", "author": "a"}),
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
            created_at: chrono::Utc::now(),
            concluded_at: None,
        };
        let payload = build_payload(&p, &d, "started", &s);
        let url = payload["deployment"]["url"].as_str().unwrap();
        // Regression: dep.url() already includes the scheme — a second
        // wrap produced "https://https://…".
        assert_eq!(url.matches("://").count(), 1, "double scheme: {url}");
        assert_eq!(url, d.url(p.slug.as_deref().unwrap(), &s));
        assert!(url.starts_with(&format!("{}://", s.url_scheme)));
        assert!(url.contains("-id-d012345"));
        d.conclusion = Some("failed".into());
        let p2 = build_payload(&p, &d, "failed", &s);
        assert_eq!(p2["event"], "deployment.failed");
    }
}
