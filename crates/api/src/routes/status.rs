//! Public status pages — `GET /api/v1/status/{slug}`.
//!
//! Opt-in per project (`config.status_page.enabled`, optional `slug`).
//! Reports each environment's live `computed_status` and a 30-day
//! uptime estimate derived from `deployment_metric` sample coverage —
//! the share of hours the serving container produced a sample.
//! Unauthenticated by design; disabled projects 404 like everything
//! else so the route reveals nothing.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use runway_core::models::Project;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn level(computed: &str) -> &'static str {
    match computed {
        "running" => "operational",
        "unhealthy" | "paused" | "degraded" => "degraded",
        "crashed" | "dead" | "missing" | "exited" | "not_found" => "outage",
        _ => "unknown",
    }
}

/// `GET /api/v1/status/{slug}` — public status for an opted-in project.
pub async fn status_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Response> {
    let project: Option<Project> = sqlx::query_as(
        "SELECT * FROM project
         WHERE status != 'deleted'
           AND (config->'status_page'->>'enabled')::boolean IS TRUE
           AND (id = $1 OR config->'status_page'->>'slug' = $1)
         LIMIT 1",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await?;
    let project = project.ok_or_else(|| ApiError::not_found("status page"))?;

    // Environment aliases → the deployment each currently serves.
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.value, a.deployment_id FROM alias a
         JOIN deployment d ON d.id = a.deployment_id
         WHERE d.project_id = $1 AND a.type = 'environment'",
    )
    .bind(&project.id)
    .fetch_all(&state.db)
    .await?;

    let env_name = |slug: &str| {
        project
            .environments
            .as_array()
            .and_then(|envs| {
                envs.iter().find(|e| {
                    e.get("slug").and_then(|s| s.as_str()) == Some(slug)
                        || e.get("id").and_then(|s| s.as_str()) == Some(slug)
                })
            })
            .and_then(|e| e.get("name").and_then(|n| n.as_str()))
            .unwrap_or(slug)
            .to_string()
    };

    let mut envs: Vec<Value> = Vec::new();
    let mut worst = "operational";
    for (slug, deployment_id) in rows {
        let Some(dep) = runway_core::deploy::get(&state.db, &deployment_id).await? else {
            continue;
        };
        let computed = dep.computed_status();
        let lvl = level(&computed);
        match lvl {
            "outage" => worst = "outage",
            "degraded" if worst != "outage" => worst = "degraded",
            "unknown" if worst == "operational" => worst = "unknown",
            _ => {}
        }
        // Uptime = share of hours in the last 30 days (since creation)
        // that produced a metric sample — i.e. the container was alive
        // and reporting.
        let hours: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT date_trunc('hour', ts))::bigint
             FROM deployment_metric
             WHERE deployment_id = $1 AND ts > now() - interval '30 days'",
        )
        .bind(&dep.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
        let span_hours = {
            let age_secs = (chrono::Utc::now() - dep.created_at).num_seconds().max(0);
            let window = age_secs.min(30 * 24 * 3600) as f64 / 3600.0;
            window.max(1.0)
        };
        let uptime = ((hours as f64 / span_hours) * 1000.0).round() / 10.0;
        envs.push(json!({
            "name": env_name(&slug),
            "slug": slug,
            "status": lvl,
            "uptime_30d": uptime.min(100.0),
            "since": dep.created_at,
        }));
    }

    Ok(Json(json!({
        "name": project.name,
        "description": project.description,
        "status": if envs.is_empty() { "unknown" } else { worst },
        "environments": envs,
        "updated_at": chrono::Utc::now(),
    }))
    .into_response())
}
