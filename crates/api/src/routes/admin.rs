//! Instance admin endpoints — currently the sign-up allowlist.
//! Port of devpush `routers/admin.py` fragment=allowlist handlers.
//! Superadmin rule is devpush's: `user.id == 1` (the first account).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// devpush `is_superadmin`: the first registered account administers
/// the instance.
pub(crate) fn require_superadmin(user: &AuthUser) -> ApiResult<()> {
    if user.user.id == 1 {
        Ok(())
    } else {
        Err(ApiError::forbidden("instance admin required"))
    }
}

/// `GET /api/v1/admin/allowlist` — all rules, newest first.
pub async fn list_allowlist(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT id, type, value, created_at::text FROM allowlist ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    let rules: Vec<Value> = rows
        .into_iter()
        .map(|(id, ty, value, created_at)| {
            json!({"id": id, "type": ty, "value": value, "created_at": created_at})
        })
        .collect();
    Ok(Json(json!({ "rules": rules })).into_response())
}

#[derive(Deserialize)]
pub struct AddAllowlistRule {
    /// `email` | `domain` | `pattern`.
    pub r#type: String,
    pub value: String,
}

fn valid_rule(ty: &str, value: &str) -> ApiResult<String> {
    let value = match ty {
        "email" => value.trim().to_lowercase(),
        "domain" => value.trim().trim_start_matches("@").to_lowercase(),
        "pattern" => value.trim().to_string(),
        _ => return Err(ApiError::bad_request("type must be email|domain|pattern")),
    };
    if value.is_empty() {
        return Err(ApiError::bad_request("value required"));
    }
    if ty == "email" && !value.contains('@') {
        return Err(ApiError::bad_request("email value must contain @"));
    }
    if ty == "domain" && (value.contains('@') || value.contains('/')) {
        return Err(ApiError::bad_request("invalid domain"));
    }
    if ty == "pattern" && regex::Regex::new(&value).is_err() {
        return Err(ApiError::bad_request("invalid regex pattern"));
    }
    Ok(value)
}

/// `POST /api/v1/admin/allowlist` — add a rule.
pub async fn add_allowlist_rule(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<AddAllowlistRule>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let value = valid_rule(&body.r#type, &body.value)?;
    let row: (i64,) =
        sqlx::query_as("INSERT INTO allowlist (type, value) VALUES ($1, $2) RETURNING id")
            .bind(&body.r#type)
            .bind(&value)
            .fetch_one(&state.db)
            .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": row.0, "type": body.r#type, "value": value})),
    )
        .into_response())
}

/// `DELETE /api/v1/admin/allowlist/{id}` — remove a rule.
pub async fn delete_allowlist_rule(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let res = sqlx::query("DELETE FROM allowlist WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("allowlist rule"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Remote Docker nodes — port of devpush `routers/admin.py`
// fragment=remote_nodes handlers + RemoteNodeService.
// ---------------------------------------------------------------------------

fn node_json(n: &runway_core::models::RemoteNode) -> Value {
    json!({
        "id": n.id,
        "name": n.name,
        "host": n.host,
        "docker_url": n.docker_url,
        "labels": n.labels,
        "status": n.status,
        "max_deployments": n.max_deployments,
        "tls": n.tls_cert.is_some(),
        "created_at": n.created_at,
    })
}

/// `GET /api/v1/admin/nodes` — all nodes, newest first.
pub async fn list_nodes(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let nodes: Vec<runway_core::models::RemoteNode> =
        sqlx::query_as("SELECT * FROM remote_node ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await?;
    let out: Vec<Value> = nodes.iter().map(node_json).collect();
    Ok(Json(json!({ "nodes": out })).into_response())
}

#[derive(Deserialize)]
pub struct CreateNode {
    pub name: String,
    pub host: String,
    /// e.g. `tcp://node.example:2375` (or `unix://`/`docker-proxy` for local).
    pub docker_url: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub max_deployments: Option<i32>,
}

/// `POST /api/v1/admin/nodes`.
pub async fn create_node(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateNode>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let name = body.name.trim();
    let host = body.host.trim();
    let url = body.docker_url.trim();
    if name.is_empty() || host.is_empty() || url.is_empty() {
        return Err(ApiError::bad_request(
            "name, host and docker_url are required",
        ));
    }
    let node: runway_core::models::RemoteNode = sqlx::query_as(
        "INSERT INTO remote_node (id, name, host, docker_url, labels,
                                  max_deployments)
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING *",
    )
    .bind(runway_core::slugify::token_hex(16))
    .bind(name)
    .bind(host)
    .bind(url)
    .bind(json!(body.labels))
    .bind(body.max_deployments.unwrap_or(10))
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(node_json(&node))).into_response())
}

/// `POST /api/v1/admin/nodes/{id}/health` — probe the node and persist
/// the observed status (devpush check_node_health + status update).
pub async fn check_node(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let node: Option<runway_core::models::RemoteNode> =
        sqlx::query_as("SELECT * FROM remote_node WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.db)
            .await?;
    let Some(node) = node else {
        return Err(ApiError::not_found("node"));
    };
    let healthy = match runway_core::docker::node_docker_client(&node) {
        Ok(client) => client.ping().await.is_ok(),
        Err(_) => false,
    };
    let status = if healthy { "online" } else { "offline" };
    let updated: runway_core::models::RemoteNode = sqlx::query_as(
        "UPDATE remote_node SET status = $2, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(&id)
    .bind(status)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(node_json(&updated)).into_response())
}

/// `DELETE /api/v1/admin/nodes/{id}` — projects keep working: the FK is
/// `ON DELETE SET NULL` so assignments silently fall back to local.
pub async fn delete_node(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let res = sqlx::query("DELETE FROM remote_node WHERE id = $1")
        .bind(&id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("node"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /api/v1/admin/nodes/{id}/tls` — generate a dedicated CA plus
/// server/client certs. Client material is stored on the node row; the
/// server bundle is returned once for the operator to install on dockerd.
pub async fn provision_node_tls(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let node: Option<runway_core::models::RemoteNode> =
        sqlx::query_as("SELECT * FROM remote_node WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.db)
            .await?;
    let Some(node) = node else {
        return Err(ApiError::not_found("node"));
    };
    let bundle = runway_core::node_tls::generate_node_tls(&node.host)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let node: runway_core::models::RemoteNode = sqlx::query_as(
        "UPDATE remote_node SET tls_ca = $2, tls_cert = $3, tls_key = $4,
         updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(&id)
    .bind(&bundle.ca_pem)
    .bind(&bundle.client_cert_pem)
    .bind(&bundle.client_key_pem)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({
        "node": node_json(&node),
        "ca_pem": bundle.ca_pem,
        "server_cert_pem": bundle.server_cert_pem,
        "server_key_pem": bundle.server_key_pem,
        "dockerd": {
            "tlsverify": true,
            "tlscacert": "/etc/docker/runway/ca.pem",
            "tlscert": "/etc/docker/runway/server.pem",
            "tlskey": "/etc/docker/runway/server-key.pem",
            "host": "tcp://0.0.0.0:2376",
        },
    }))
    .into_response())
}

/// `DELETE /api/v1/admin/nodes/{id}/tls` — drop the stored client
/// material; the node falls back to its plain `docker_url`.
pub async fn clear_node_tls(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let res = sqlx::query(
        "UPDATE remote_node SET tls_ca = NULL, tls_cert = NULL, tls_key = NULL,
         updated_at = now() WHERE id = $1",
    )
    .bind(&id)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("node"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Job queue — instance-level inspection + manual retry of failed work.
// Beyond devpush (its admin surface has no queue view).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct JobsQuery {
    status: Option<String>,
    limit: Option<i64>,
}

type JobRow = (
    i64,
    String,
    String,
    i32,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
);

/// `GET /api/v1/admin/jobs[?status=failed&limit=50]` — queue counts by
/// status plus the newest matching rows.
pub async fn list_jobs(
    user: AuthUser,
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<JobsQuery>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let counts: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*)::bigint FROM job GROUP BY status ORDER BY status")
            .fetch_all(&state.db)
            .await?;
    let rows: Vec<JobRow> = match &q.status {
        Some(s) => {
            sqlx::query_as(
                "SELECT id, kind, status, attempts, last_error, created_at, updated_at
                 FROM job WHERE status = $1 ORDER BY id DESC LIMIT $2",
            )
            .bind(s)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT id, kind, status, attempts, last_error, created_at, updated_at
                 FROM job ORDER BY id DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
    };
    Ok(Json(json!({
        "counts": counts.iter().map(|(s, n)| json!({ "status": s, "count": n })).collect::<Vec<_>>(),
        "jobs": rows.iter().map(|r| json!({
            "id": r.0, "kind": r.1, "status": r.2, "attempts": r.3,
            "last_error": r.4, "created_at": r.5, "updated_at": r.6,
        })).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// `POST /api/v1/admin/jobs/{id}/retry` — requeue a failed job.
/// Resets attempts so max_attempts doesn't immediately re-fail it.
pub async fn retry_job(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let res = sqlx::query(
        "UPDATE job SET status = 'pending', attempts = 0, locked_at = NULL,
         last_error = 'manual retry', updated_at = now()
         WHERE id = $1 AND status = 'failed'",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("failed job"));
    }
    Ok(Json(json!({ "ok": true })).into_response())
}
