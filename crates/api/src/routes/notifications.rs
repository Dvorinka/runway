//! Notifications, audit reads, and the deploy button —
//! devpush `user.py` notifications + `project.py` /deploy.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
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

/// Exclusive cursor. Empty means the newest page. Zero and negative are
/// rejected so a bad query cannot scan from the other end of the table.
pub(crate) fn normalize_before(raw: Option<i64>) -> ApiResult<Option<i64>> {
    match raw {
        None => Ok(None),
        Some(n) if n > 0 => Ok(Some(n)),
        Some(_) => Err(ApiError::bad_request("before_id must be a positive id")),
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
) -> ApiResult<(Vec<serde_json::Value>, bool)> {
    let team = clean_id(team_id);
    let action = normalize_action(action)?;
    let before = normalize_before(before_id)?;
    let limit = clamp_audit_limit(limit);
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
         AND ($6::bigint IS NULL OR a.id < $6)
         -- shortcut: id cursor assumes created_at follows id, upgrade to a (created_at, id) keyset if rows are backdated.
         ORDER BY a.created_at DESC, a.id DESC
         LIMIT $5",
    )
    .bind(superadmin)
    .bind(user_id)
    .bind(&team)
    .bind(&action)
    .bind(fetch)
    .bind(before)
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
    )
    .await?;
    Ok(Json(json!({ "entries": entries, "has_more": has_more })).into_response())
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
    fn before_id_rejects_non_positive() {
        assert_eq!(normalize_before(None).ok().flatten(), None);
        assert_eq!(normalize_before(Some(12)).ok().flatten(), Some(12));
        assert!(normalize_before(Some(0)).is_err());
        assert!(normalize_before(Some(-1)).is_err());
    }
}
