//! Speed insights — real-user monitoring.
//!
//! `POST /_runway-rum` (and `/api/v1/rum`): public beacon hit by the
//! injected script on deployed sites. Unauthenticated — the project is
//! resolved from the Host header, and events are only stored when the
//! project has `config.speed_insights.enabled`, so the endpoint can't
//! be used to write against arbitrary projects.
//!
//! `GET /api/v1/projects/{id}/speed`: authenticated aggregates for the
//! dashboard — p75 of each vital plus per-path breakdown.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::models::Project;
use runway_core::rum;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct Beacon {
    /// Page path (m.p).
    p: Option<String>,
    lcp: Option<f64>,
    fcp: Option<f64>,
    inp: Option<f64>,
    cls: Option<f64>,
    ttfb: Option<f64>,
}

/// Resolve the project serving `host`: alias subdomain on the deploy
/// domain first, then an active custom domain.
async fn project_for_host(state: &AppState, host: &str) -> ApiResult<Option<Project>> {
    let suffix = format!(".{}", state.settings.deploy_domain);
    if let Some(sub) = host.strip_suffix(&suffix) {
        let project: Option<Project> = sqlx::query_as(
            "SELECT p.* FROM alias a
             JOIN deployment d ON d.id = a.deployment_id
             JOIN project p ON p.id = d.project_id
             WHERE a.subdomain = $1 AND p.status != 'deleted' LIMIT 1",
        )
        .bind(sub)
        .fetch_optional(&state.db)
        .await?;
        if project.is_some() {
            return Ok(project);
        }
    }
    let project: Option<Project> = sqlx::query_as(
        "SELECT p.* FROM project p
         JOIN domain d ON d.project_id = p.id
         WHERE d.hostname = $1 AND d.status = 'active' AND p.status != 'deleted'
         LIMIT 1",
    )
    .bind(host)
    .fetch_optional(&state.db)
    .await?;
    Ok(project)
}

/// `POST /_runway-rum` — beacon. Always 204; invalid input is dropped
/// silently (beacons shouldn't surface errors to end users).
pub async fn beacon(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Beacon>,
) -> Response {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(256)
        .collect::<String>();

    let Ok(Some(project)) = project_for_host(&state, &host).await else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let enabled = project
        .config
        .get("speed_insights")
        .and_then(|v| v.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return StatusCode::NO_CONTENT.into_response();
    }

    let path = rum::sanitize_path(body.p.as_deref().unwrap_or("/"));
    let _ = sqlx::query(
        "INSERT INTO rum_event (project_id, host, path, lcp, fcp, inp, cls, ttfb, ua)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&project.id)
    .bind(host)
    .bind(path)
    .bind(body.lcp.and_then(rum::clamp_metric))
    .bind(body.fcp.and_then(rum::clamp_metric))
    .bind(body.inp.and_then(rum::clamp_metric))
    .bind(body.cls.and_then(rum::clamp_metric))
    .bind(body.ttfb.and_then(rum::clamp_metric))
    .bind(ua)
    .execute(&state.db)
    .await;
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
pub struct SpeedQuery {
    days: Option<i64>,
}

/// `GET /api/v1/projects/{id}/speed` — p75 aggregates + per-path rows.
pub async fn speed(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SpeedQuery>,
) -> ApiResult<Response> {
    let project: Option<Project> = sqlx::query_as(
        "SELECT p.* FROM project p
         JOIN team_member tm ON tm.team_id = p.team_id
         WHERE tm.user_id = $1 AND p.id = $2 AND p.status != 'deleted'",
    )
    .bind(user.user.id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    let project = project.ok_or_else(|| ApiError::not_found("project"))?;
    let days = q.days.unwrap_or(7).clamp(1, 90);

    type Row = (
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        i64,
    );
    let (lcp, fcp, inp, cls, ttfb, views): Row = sqlx::query_as(
        "SELECT percentile_cont(0.75) WITHIN GROUP (ORDER BY lcp),
                percentile_cont(0.75) WITHIN GROUP (ORDER BY fcp),
                percentile_cont(0.75) WITHIN GROUP (ORDER BY inp),
                percentile_cont(0.75) WITHIN GROUP (ORDER BY cls),
                percentile_cont(0.75) WITHIN GROUP (ORDER BY ttfb),
                count(*)
         FROM rum_event
         WHERE project_id = $1 AND ts > now() - make_interval(days => $2)",
    )
    .bind(&project.id)
    .bind(days as i32)
    .fetch_one(&state.db)
    .await?;

    let paths: Vec<Value> = sqlx::query_scalar(
        "SELECT json_build_object('path', path, 'views', count(*),
           'lcp', percentile_cont(0.75) WITHIN GROUP (ORDER BY lcp))
         FROM rum_event
         WHERE project_id = $1 AND ts > now() - make_interval(days => $2)
         GROUP BY path ORDER BY count(*) DESC LIMIT 10",
    )
    .bind(&project.id)
    .bind(days as i32)
    .fetch_all(&state.db)
    .await?;

    let series: Vec<Value> = sqlx::query_scalar(
        "SELECT json_build_object('day', d, 'views', count(*),
           'lcp', percentile_cont(0.75) WITHIN GROUP (ORDER BY lcp))
         FROM (SELECT date_trunc('day', ts)::date AS d, lcp
               FROM rum_event
               WHERE project_id = $1 AND ts > now() - make_interval(days => $2)) e
         GROUP BY d ORDER BY d",
    )
    .bind(&project.id)
    .bind(days as i32)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({
        "days": days,
        "views": views,
        "p75": { "lcp": lcp, "fcp": fcp, "inp": inp, "cls": cls, "ttfb": ttfb },
        "paths": paths,
        "series": series,
    }))
    .into_response())
}
