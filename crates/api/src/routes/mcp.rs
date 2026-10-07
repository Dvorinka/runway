//! Real MCP endpoint — JSON-RPC 2.0 over `POST /api/mcp`.
//! Auth: `Authorization: Bearer ak_...` (same extractor as REST).
//! Read-mostly tool set + `redeploy_deployment` for agent-driven ops.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

fn tools() -> Value {
    json!([
        tool(
            "list_projects",
            "List all projects the API key can access",
            json!({"type": "object", "properties": {}})
        ),
        tool(
            "get_project",
            "Project details incl. config, environments, domains",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"}}})
        ),
        tool(
            "list_deployments",
            "Recent deployments for a project",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "limit": {"type": "integer", "default": 20}}})
        ),
        tool(
            "get_deployment",
            "Deployment details incl. status, conclusion, URLs",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"}}})
        ),
        tool(
            "get_deployment_logs",
            "Tail of a deployment's build/runtime log",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"},
                                  "tail": {"type": "integer", "default": 200}}})
        ),
        tool(
            "list_domains",
            "Custom domains configured on a project",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"}}})
        ),
        tool(
            "redeploy_deployment",
            "Create a new deployment reusing the same commit",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"}}})
        ),
    ])
}

fn ok(id: &Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_err(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    }))
    .into_response()
}

/// `POST /api/mcp` — JSON-RPC dispatcher.
pub async fn rpc(
    user: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> ApiResult<Response> {
    let method = req["method"].as_str().unwrap_or("");
    let id = &req["id"];
    // Notifications carry no id — acknowledge silently.
    if method.starts_with("notifications/") || id.is_null() {
        return Ok(axum::http::StatusCode::ACCEPTED.into_response());
    }

    match method {
        "initialize" => Ok(ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "runway", "version": env!("CARGO_PKG_VERSION") },
            }),
        )),
        "ping" => Ok(ok(id, json!({}))),
        "tools/list" => Ok(ok(id, json!({ "tools": tools() }))),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"]["arguments"].clone();
            match call(&state, user.user.id, name, &args).await {
                Ok(result) => Ok(ok(
                    id,
                    json!({
                        "content": [{ "type": "text",
                                      "text": serde_json::to_string_pretty(&result).unwrap_or_default() }],
                        "isError": false,
                    }),
                )),
                Err(e) => Ok(ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": e.1 }],
                        "isError": true,
                    }),
                )),
            }
        }
        _ => Ok(rpc_err(id, -32601, &format!("method '{method}' not found"))),
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> ApiResult<&'a str> {
    args[key]
        .as_str()
        .ok_or_else(|| ApiError::bad_request(format!("missing parameter '{key}'")))
}

async fn call(state: &AppState, user_id: i64, name: &str, args: &Value) -> ApiResult<Value> {
    match name {
        "list_projects" => {
            let projects: Vec<runway_core::models::Project> = sqlx::query_as(
                "SELECT p.* FROM project p
                 JOIN team_member tm ON tm.team_id = p.team_id
                 WHERE tm.user_id = $1 AND p.status != 'deleted'
                 ORDER BY p.created_at DESC",
            )
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
            Ok(json!(projects
                .iter()
                .map(|p| json!({
                    "id": p.id, "name": p.name, "slug": p.slug,
                    "repo": p.repo_full_name, "branch": p.repo_branch,
                    "config": p.config,
                }))
                .collect::<Vec<_>>()))
        }
        "get_project" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let domains: Vec<runway_core::models::Domain> =
                sqlx::query_as("SELECT * FROM domain WHERE project_id = $1 ORDER BY id")
                    .bind(&p.id)
                    .fetch_all(&state.db)
                    .await?;
            Ok(json!({ "project": p, "domains": domains }))
        }
        "list_deployments" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let limit = args["limit"].as_i64().unwrap_or(20).clamp(1, 100);
            let deps: Vec<runway_core::models::Deployment> = sqlx::query_as(
                "SELECT * FROM deployment WHERE project_id = $1
                 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(&p.id)
            .bind(limit)
            .fetch_all(&state.db)
            .await?;
            Ok(json!(deps
                .iter()
                .map(|d| json!({
                    "id": d.id, "status": d.status, "conclusion": d.conclusion,
                    "branch": d.branch, "commit_sha": d.commit_sha,
                    "trigger": d.trigger, "created_at": d.created_at,
                    "url": d.url(p.slug.as_deref().unwrap_or(&p.id), &state.settings),
                }))
                .collect::<Vec<_>>()))
        }
        "get_deployment" => {
            let (d, p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            Ok(json!({
                "id": d.id, "project_id": d.project_id, "status": d.status,
                "conclusion": d.conclusion, "branch": d.branch,
                "commit_sha": d.commit_sha, "commit_meta": d.commit_meta,
                "trigger": d.trigger, "error": d.error,
                "computed_status": d.computed_status(),
                "url": d.url(p.slug.as_deref().unwrap_or(&p.id), &state.settings),
                "created_at": d.created_at, "concluded_at": d.concluded_at,
            }))
        }
        "get_deployment_logs" => {
            let (d, _p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let tail = args["tail"].as_u64().unwrap_or(200).clamp(1, 5000) as usize;
            let lines = state.logs.tail(&d.id, tail).await.unwrap_or_default();
            Ok(json!({ "deployment_id": d.id, "lines": lines }))
        }
        "list_domains" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let domains: Vec<runway_core::models::Domain> =
                sqlx::query_as("SELECT * FROM domain WHERE project_id = $1 ORDER BY id")
                    .bind(&p.id)
                    .fetch_all(&state.db)
                    .await?;
            Ok(json!(domains))
        }
        "redeploy_deployment" => {
            let (d, p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let info = runway_core::deploy::CommitInfo {
                sha: d.commit_sha.clone(),
                message: d
                    .commit_meta
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                author: d
                    .commit_meta
                    .get("author")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                timestamp: None,
            };
            let extra = d
                .config
                .get("source_archive")
                .cloned()
                .map(|a| json!({ "source_archive": a }));
            let new_dep = runway_core::deploy::create(
                &state.db,
                &state.bus,
                &state.crypto,
                &p,
                &d.branch,
                &info,
                "api",
                Some(user_id),
                extra,
            )
            .await?;
            Ok(json!({ "deployment_id": new_dep.id, "status": new_dep.status }))
        }
        _ => Err(ApiError::bad_request(format!("unknown tool '{name}'"))),
    }
}

async fn project_for(
    state: &AppState,
    user_id: i64,
    id: &str,
) -> ApiResult<runway_core::models::Project> {
    let p: Option<runway_core::models::Project> = sqlx::query_as(
        "SELECT p.* FROM project p
         JOIN team_member tm ON tm.team_id = p.team_id
         WHERE tm.user_id = $1 AND p.id = $2 AND p.status != 'deleted'",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    p.ok_or_else(|| ApiError::not_found("project"))
}

async fn deployment_for(
    state: &AppState,
    user_id: i64,
    id: &str,
) -> ApiResult<(
    runway_core::models::Deployment,
    runway_core::models::Project,
)> {
    let d = runway_core::deploy::get(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("deployment"))?;
    let p = project_for(state, user_id, &d.project_id).await?;
    Ok((d, p))
}
