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
            "deployment_stats",
            "Live container resource snapshot (cpu %, memory, network, pids)",
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
        tool(
            "deploy_project",
            "Trigger a new deployment on a project (latest commit, or the last uploaded archive for remote-less projects)",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "branch": {"type": "string"}}})
        ),
        tool(
            "cancel_deployment",
            "Cancel a running deployment and stop its container",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"}}})
        ),
        tool(
            "rollback_environment",
            "Roll an environment back to its previous deployment",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "environment_id": {"type": "string", "default": "prod"}}})
        ),
        tool(
            "list_env",
            "List a project's env vars (values masked)",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"}}})
        ),
        tool(
            "set_env",
            "Upsert or delete env vars: [{key, value?, environment?, delete?}]",
            json!({"type": "object", "required": ["project_id", "vars"],
                   "properties": {"project_id": {"type": "string"},
                                  "vars": {"type": "array"}}})
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
        "deployment_stats" => {
            let (d, _p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let Some(cid) = d.container_id.clone() else {
                return Ok(json!({ "running": false }));
            };
            let docker = match d.remote_node_id.as_deref() {
                Some(nid) => match runway_core::docker::node_client(&state.db, nid).await {
                    Some(c) => c,
                    None => return Ok(json!({ "running": false })),
                },
                None => match &state.docker {
                    Some(d) => d.clone(),
                    None => return Ok(json!({ "running": false })),
                },
            };
            match runway_core::docker::stats_snapshot(&docker, &cid).await {
                Some(s) => Ok(json!({
                    "running": true, "cpu_pct": s.cpu_pct,
                    "mem_used": s.mem_used, "mem_limit": s.mem_limit,
                    "net_rx": s.net_rx, "net_tx": s.net_tx, "pids": s.pids,
                })),
                None => Ok(json!({ "running": false })),
            }
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
        "deploy_project" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let branch = args["branch"]
                .as_str()
                .filter(|b| !b.is_empty())
                .unwrap_or(if p.repo_branch.is_empty() {
                    "main"
                } else {
                    &p.repo_branch
                })
                .to_string();
            let has_remote = match p.repo_provider.as_str() {
                "github" | "github_enterprise" => p.github_installation_id.is_some(),
                "gitea" | "forgejo" => p.gitea_connection_id.is_some(),
                "gitlab" => p.gitlab_connection_id.is_some(),
                "bitbucket" => p.bitbucket_connection_id.is_some(),
                _ => false,
            };
            let dep = if has_remote {
                crate::routes::deployments::trigger_deployment(
                    state,
                    &p,
                    &branch,
                    "api",
                    Some(user_id),
                )
                .await?
            } else {
                let prev: Option<runway_core::models::Deployment> = sqlx::query_as(
                    "SELECT * FROM deployment WHERE project_id = $1 AND branch = $2 \
                     AND config->>'source_archive' IS NOT NULL \
                     ORDER BY created_at DESC LIMIT 1",
                )
                .bind(&p.id)
                .bind(&branch)
                .fetch_optional(&state.db)
                .await?;
                let Some(prev) = prev else {
                    return Err(ApiError::bad_request(
                        "project has no git remote — deploy via `runway deploy` first",
                    ));
                };
                crate::routes::deployments::redeploy_dep(state, user_id, &prev, &p).await?
            };
            Ok(json!({ "deployment_id": dep.id, "status": dep.status, "branch": dep.branch }))
        }
        "cancel_deployment" => {
            let (d, _p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let docker = state
                .docker
                .as_ref()
                .ok_or_else(|| ApiError::bad_request("docker unavailable"))?;
            runway_core::deploy::cancel(&state.db, &state.bus, &state.settings, docker, &d)
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(json!({ "deployment_id": d.id, "conclusion": "canceled" }))
        }
        "rollback_environment" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let env = args["environment_id"].as_str().unwrap_or("prod");
            let alias =
                runway_core::deploy::rollback(&state.db, &state.bus, &state.settings, &p, env)
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(json!({ "environment_id": env, "deployment_id": alias.deployment_id }))
        }
        "list_env" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let vars = p
                .env_vars(&state.crypto)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(json!(vars
                .iter()
                .map(|v| json!({
                    "key": v.key,
                    "value": if v.value.len() <= 4 { "****".into() }
                             else { format!("{}…{}", &v.value[..2], &v.value[v.value.len()-2..]) },
                    "environment": v.environment,
                }))
                .collect::<Vec<_>>()))
        }
        "set_env" => {
            let mut p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let items = args["vars"]
                .as_array()
                .ok_or_else(|| ApiError::bad_request("vars must be an array"))?;
            let mut vars = p
                .env_vars(&state.crypto)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            for it in items {
                let key = it["key"].as_str().unwrap_or("");
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(ApiError::bad_request(format!("invalid env key '{key}'")));
                }
                let environment = it["environment"].as_str().map(str::to_string);
                if let Some(env) = &environment {
                    if p.environments()
                        .iter()
                        .all(|e| &e.slug != env && &e.id != env)
                    {
                        return Err(ApiError::bad_request(format!(
                            "unknown environment '{env}'"
                        )));
                    }
                }
                vars.retain(|v| !(v.key == key && v.environment == environment));
                if it["delete"].as_bool() != Some(true) {
                    let value = it["value"]
                        .as_str()
                        .ok_or_else(|| ApiError::bad_request("value required unless delete=true"))?
                        .to_string();
                    vars.push(runway_core::models::EnvVar {
                        key: key.to_string(),
                        value,
                        environment,
                    });
                }
            }
            p.set_env_vars(&state.crypto, &vars)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            sqlx::query("UPDATE project SET env_vars = $1, updated_at = now() WHERE id = $2")
                .bind(&p.env_vars)
                .bind(&p.id)
                .execute(&state.db)
                .await?;
            Ok(json!({ "ok": true, "count": vars.len() }))
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
