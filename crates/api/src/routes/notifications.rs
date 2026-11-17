//! Notifications, audit reads, and the deploy button —
//! devpush `user.py` notifications + `project.py` /deploy.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct MarkReadBody {
    pub id: Option<i64>,
}

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
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

/// `POST /api/v1/notifications/mark-read` — one (`{id}`) or all read
/// (empty body, devpush parity).
pub async fn mark_read(
    user: AuthUser,
    State(state): State<AppState>,
    body: Option<Json<MarkReadBody>>,
) -> ApiResult<Response> {
    match body.and_then(|b| b.0.id) {
        Some(id) => {
            sqlx::query("UPDATE notification SET read = TRUE WHERE id = $1 AND user_id = $2")
                .bind(id)
                .bind(user.user.id)
                .execute(&state.db)
                .await?;
        }
        None => {
            sqlx::query("UPDATE notification SET read = TRUE WHERE user_id = $1 AND NOT read")
                .bind(user.user.id)
                .execute(&state.db)
                .await?;
        }
    }
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

/// Accept an action filter. Empty means every action. Anything else must
/// be a short identifier such as `deployment.create`.
pub(crate) fn normalize_action(raw: Option<&str>) -> ApiResult<Option<String>> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let ok = s.len() <= 50
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
    if !ok {
        return Err(ApiError::bad_request(
            "action filter must be a short identifier",
        ));
    }
    Ok(Some(s.to_string()))
}

pub(crate) fn clamp_audit_limit(n: Option<i64>) -> i64 {
    n.unwrap_or(100).clamp(1, 200)
}

pub(crate) struct AuditCursor {
    pub id: i64,
    pub at: DateTime<Utc>,
}

/// Page boundary. Both halves are required so a backdated row is not skipped
/// or repeated. Empty means the newest page.
pub(crate) fn normalize_cursor(
    id: Option<i64>,
    at: Option<&str>,
) -> ApiResult<Option<AuditCursor>> {
    let at = at.map(str::trim).filter(|s| !s.is_empty());
    match (id, at) {
        (None, None) => Ok(None),
        (Some(id), Some(raw)) if id > 0 => {
            let at = DateTime::parse_from_rfc3339(raw)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|_| ApiError::bad_request("before_at must be an RFC3339 timestamp"))?;
            Ok(Some(AuditCursor { id, at }))
        }
        (Some(id), _) if id <= 0 => Err(ApiError::bad_request("before_id must be a positive id")),
        _ => Err(ApiError::bad_request(
            "before_id and before_at are set together",
        )),
    }
}

fn clean_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Rows the caller may see.
///
/// The instance admin (first account) sees every row, matching devpush's
/// settings audit list. A team owner or admin sees that team's rows. A
/// member sees only the rows they performed on a team they belong to.
pub async fn visible_audit(
    state: &AppState,
    user_id: i64,
    team_id: Option<&str>,
    action: Option<&str>,
    limit: Option<i64>,
    before_id: Option<i64>,
    before_at: Option<&str>,
) -> ApiResult<(Vec<serde_json::Value>, bool)> {
    visible_audit_clamped(
        state,
        user_id,
        team_id,
        action,
        clamp_audit_limit(limit),
        before_id,
        before_at,
    )
    .await
}

/// Export reads further back than the page cap — still bounded so one
/// request cannot stream the whole table.
pub(crate) fn clamp_audit_export_limit(n: Option<i64>) -> i64 {
    n.unwrap_or(10_000).clamp(1, 10_000)
}

/// Every row the caller may see, up to the export bound. Powers
/// `GET /api/v1/audit/export`.
pub async fn visible_audit_export(
    state: &AppState,
    user_id: i64,
    team_id: Option<&str>,
    action: Option<&str>,
    limit: Option<i64>,
) -> ApiResult<Vec<serde_json::Value>> {
    let (rows, _) = visible_audit_clamped(
        state,
        user_id,
        team_id,
        action,
        clamp_audit_export_limit(limit),
        None,
        None,
    )
    .await?;
    Ok(rows)
}

async fn visible_audit_clamped(
    state: &AppState,
    user_id: i64,
    team_id: Option<&str>,
    action: Option<&str>,
    limit: i64,
    before_id: Option<i64>,
    before_at: Option<&str>,
) -> ApiResult<(Vec<serde_json::Value>, bool)> {
    let team = clean_id(team_id);
    let action = normalize_action(action)?;
    let cursor = normalize_cursor(before_id, before_at)?;
    let before_id = cursor.as_ref().map(|c| c.id);
    let before_at = cursor.as_ref().map(|c| c.at);
    // One extra row tells the caller whether another page exists.
    let fetch = limit + 1;
    let superadmin = crate::routes::admin::is_superadmin_id(user_id);
    if let Some(ref tid) = team {
        if !superadmin {
            let role: Option<String> = sqlx::query_scalar(
                "SELECT role FROM team_member WHERE team_id = $1 AND user_id = $2",
            )
            .bind(tid)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
            if role.is_none() {
                return Err(ApiError::forbidden("not a member of that team"));
            }
        }
    }
    let mut rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT json_build_object(
           'id', a.id, 'action', a.action, 'user_id', a.user_id,
           'username', u.username, 'team_id', a.team_id, 'team_name', t.name,
           'project_id', a.project_id, 'project_name', p.name,
           'resource_type', a.resource_type,
           'resource_id', a.resource_id, 'detail', a.detail,
           'created_at', a.created_at)
         FROM audit_log a
         LEFT JOIN \"user\" u ON u.id = a.user_id
         LEFT JOIN team t ON t.id = a.team_id
         LEFT JOIN project p ON p.id = a.project_id
         WHERE (
           $1::bool
           OR a.team_id IN (
             SELECT team_id FROM team_member
             WHERE user_id = $2 AND role IN ('owner', 'admin')
           )
           OR (
             a.user_id = $2
             AND a.team_id IN (SELECT team_id FROM team_member WHERE user_id = $2)
           )
         )
         AND ($3::varchar IS NULL OR a.team_id = $3)
         AND ($4::varchar IS NULL OR a.action = $4)
         AND (
           $6::timestamptz IS NULL
           OR a.created_at < $6
           OR (a.created_at = $6 AND a.id < $7)
         )
         ORDER BY a.created_at DESC, a.id DESC
         LIMIT $5",
    )
    .bind(superadmin)
    .bind(user_id)
    .bind(&team)
    .bind(&action)
    .bind(fetch)
    .bind(before_at)
    .bind(before_id)
    .fetch_all(&state.db)
    .await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    Ok((rows, has_more))
}

#[derive(Deserialize)]
pub struct AuditListQuery {
    pub team_id: Option<String>,
    pub action: Option<String>,
    pub limit: Option<i64>,
    pub before_id: Option<i64>,
    pub before_at: Option<String>,
}

/// `GET /api/v1/audit` — cross-team audit the caller is allowed to see.
pub async fn list_audit(
    user: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<AuditListQuery>,
) -> ApiResult<Response> {
    let (entries, has_more) = visible_audit(
        &state,
        user.user.id,
        q.team_id.as_deref(),
        q.action.as_deref(),
        q.limit,
        q.before_id,
        q.before_at.as_deref(),
    )
    .await?;
    Ok(Json(json!({ "entries": entries, "has_more": has_more })).into_response())
}

#[derive(Deserialize)]
pub struct AuditExportQuery {
    pub team_id: Option<String>,
    pub action: Option<String>,
    pub format: Option<String>,
    pub limit: Option<i64>,
}

fn csv_field(v: Option<&serde_json::Value>) -> String {
    let s = match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// `GET /api/v1/audit/export` — the rows `GET /api/v1/audit` pages through,
/// unpaged up to the export bound, as CSV (default) or JSON
/// (`?format=json`). Same visibility rules; `detail` stays a JSON string.
pub async fn export_audit(
    user: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<AuditExportQuery>,
) -> ApiResult<Response> {
    let rows = visible_audit_export(
        &state,
        user.user.id,
        q.team_id.as_deref(),
        q.action.as_deref(),
        q.limit,
    )
    .await?;
    if q.format.as_deref() == Some("json") {
        return Ok(Json(json!({ "entries": rows })).into_response());
    }
    let mut out = String::from(
        "id,created_at,action,username,user_id,team_id,team_name,project_id,project_name,resource_type,resource_id,detail\n",
    );
    for r in &rows {
        let f = |k: &str| csv_field(r.get(k));
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}\n",
            f("id"),
            f("created_at"),
            f("action"),
            f("username"),
            f("user_id"),
            f("team_id"),
            f("team_name"),
            f("project_id"),
            f("project_name"),
            f("resource_type"),
            f("resource_id"),
            f("detail"),
        ));
    }
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"audit-export.csv\"".to_string(),
            ),
        ],
        out.into_bytes(),
    )
        .into_response())
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
            h if h.contains("forgejo") => "forgejo",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_filter_accepts_identifiers_only() {
        assert_eq!(
            normalize_action(Some("deployment.create"))
                .ok()
                .flatten()
                .as_deref(),
            Some("deployment.create")
        );
        assert!(normalize_action(None).ok().flatten().is_none());
        assert!(normalize_action(Some("  ")).ok().flatten().is_none());
        assert!(normalize_action(Some("drop table;")).is_err());
        assert!(normalize_action(Some(&"a".repeat(51))).is_err());
    }

    #[test]
    fn audit_limit_clamps() {
        assert_eq!(clamp_audit_limit(None), 100);
        assert_eq!(clamp_audit_limit(Some(0)), 1);
        assert_eq!(clamp_audit_limit(Some(9_000)), 200);
        assert_eq!(clamp_audit_limit(Some(25)), 25);
    }

    #[test]
    fn cursor_needs_a_positive_id_and_a_timestamp() {
        let ok = normalize_cursor(Some(12), Some("2026-11-12T14:16:25.664059+00:00"));
        assert_eq!(ok.ok().flatten().as_ref().map(|c| c.id), Some(12));
        assert!(normalize_cursor(None, None).ok().flatten().is_none());
        assert!(normalize_cursor(Some(0), Some("2026-11-12T14:16:25Z")).is_err());
        assert!(normalize_cursor(Some(-1), Some("2026-11-12T14:16:25Z")).is_err());
        assert!(normalize_cursor(Some(12), None).is_err());
        assert!(normalize_cursor(None, Some("2026-11-12T14:16:25Z")).is_err());
        assert!(normalize_cursor(Some(12), Some("yesterday")).is_err());
    }

    #[test]
    fn export_limit_allows_deeper_reads() {
        assert_eq!(clamp_audit_export_limit(None), 10_000);
        assert_eq!(clamp_audit_export_limit(Some(0)), 1);
        assert_eq!(clamp_audit_export_limit(Some(50_000)), 10_000);
        assert_eq!(clamp_audit_export_limit(Some(500)), 500);
    }

    #[test]
    fn csv_field_quotes_only_when_needed() {
        assert_eq!(csv_field(Some(&json!("plain"))), "plain");
        assert_eq!(csv_field(Some(&json!(42))), "42");
        assert_eq!(csv_field(None), "");
        assert_eq!(csv_field(Some(&json!("a,b"))), "\"a,b\"");
        assert_eq!(csv_field(Some(&json!("say \"hi\""))), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field(Some(&json!("line\nbreak"))), "\"line\nbreak\"");
    }
}
