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

pub const WEBHOOK_EVENTS: [&str; 5] = ["started", "succeeded", "failed", "canceled", "skipped"];

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
            "url": format!("{}://{}", settings.url_scheme,
                dep.url(project.slug.as_deref().unwrap_or(&project.id), settings)),
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
