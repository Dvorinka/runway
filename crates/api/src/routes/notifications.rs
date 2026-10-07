//! Notifications, audit reads, and the deploy button —
//! devpush `user.py` notifications + `project.py` /deploy.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::state::AppState;

/// `GET /api/v1/notifications` — newest 50 + unread count.
pub async fn list(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT json_build_object(
           'id', id, 'type', type, 'title', title, 'body', body,
           'link', link, 'read', read, 'created_at', created_at,
           'team_id', team_id, 'project_id', project_id)
         FROM notification WHERE user_id = $1
         ORDER BY created_at DESC LIMIT 50",
    )
    .bind(user.user.id)
    .fetch_all(&state.db)
    .await?;
    let (unread,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM notification WHERE user_id = $1 AND NOT read")
            .bind(user.user.id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(json!({ "notifications": rows, "unread": unread })).into_response())
}

/// `POST /api/v1/notifications/mark-read` — all read (devpush parity).
pub async fn mark_read(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    sqlx::query("UPDATE notification SET read = TRUE WHERE user_id = $1 AND NOT read")
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })).into_response())
}

/// `GET /api/v1/teams/{id}/audit` — admin+ only, newest 200.
pub async fn team_audit(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> ApiResult<Response> {
    let (_, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT json_build_object(
           'id', a.id, 'action', a.action, 'user_id', a.user_id,
           'username', u.username, 'project_id', a.project_id,
           'resource_type', a.resource_type, 'resource_id', a.resource_id,
           'detail', a.detail, 'created_at', a.created_at)
         FROM audit_log a LEFT JOIN \"user\" u ON u.id = a.user_id
         WHERE a.team_id = $1
         ORDER BY a.created_at DESC LIMIT 200",
    )
    .bind(&team_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "entries": rows })).into_response())
}

/// `GET /api/deploy?repo_url=...&branch=...&provider=github` —
/// "Deploy on Runway" button target (devpush `/deploy`). Returns the
/// prefill the SPA consumes; redirect happens client-side.
#[derive(Deserialize)]
pub struct DeployQuery {
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub provider: Option<String>,
}

pub async fn deploy_button(Query(q): Query<DeployQuery>) -> Response {
    let mut repo_url = q.repo_url.unwrap_or_default();
    let mut provider = q.provider.unwrap_or_else(|| "github".into());
    // Infer provider from the host when not specified (devpush parity).
    if !repo_url.is_empty() {
        let host = repo_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_lowercase();
        provider = match host.as_str() {
            h if h.contains("gitlab") => "gitlab",
            h if h.contains("bitbucket") => "bitbucket",
            h if h.contains("gitea") => "gitea",
            _ => "github",
        }
        .to_string();
    }
    // Normalize SSH-style git@host:owner/repo.git → https URL.
    if let Some(rest) = repo_url.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            repo_url = format!("https://{host}/{path}");
        }
    }
    repo_url = repo_url.trim_end_matches(".git").to_string();
    Json(json!({
        "repo_url": repo_url,
        "branch": q.branch,
        "provider": provider,
    }))
    .into_response()
}
