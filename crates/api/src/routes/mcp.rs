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
            "get_project_usage",
            "Usage metering for a project: deploy counts, build seconds, artifact/image bytes over N days",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "days": {"type": "integer", "default": 30}}})
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
                                  "branch": {"type": "string"},
                                  "idempotency_key": {"type": "string",
                                     "description": "dedupe key — same key returns the existing deployment instead of queueing a duplicate"}}})
        ),
        tool(
            "cancel_deployment",
            "Cancel a running deployment and stop its container",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"}}})
        ),
        tool(
            "skip_deployment",
            "Skip a queued deployment so it never runs",
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
            "promote_deployment",
            "Point an environment's alias at a successful deployment",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"},
                                  "environment_id": {"type": "string", "default": "prod"}}})
        ),
        tool(
            "promotion_diff",
            "Preview what promoting a deployment would change. Env and config values are omitted. Does not promote.",
            json!({"type": "object", "required": ["deployment_id"],
                   "properties": {"deployment_id": {"type": "string"},
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
        tool(
            "list_audit",
            "Audit log the caller may see. Instance admin sees every team. Team admins see their teams. Members see their own actions. Values of secrets are never included.",
            json!({"type": "object",
                   "properties": {"team_id": {"type": "string"},
                                  "action": {"type": "string"},
                                  "limit": {"type": "integer"},
                                  "before_id": {"type": "integer"},
                                  "before_at": {"type": "string"}}})
        ),
        tool(
            "export_audit",
            "Export every audit row the caller may see, unpaged up to the 10000-row bound. Same visibility rules as list_audit; has_more reports truncation.",
            json!({"type": "object",
                   "properties": {"team_id": {"type": "string"},
                                  "action": {"type": "string"},
                                  "limit": {"type": "integer"}}})
        ),
        tool(
            "list_cron_jobs",
            "List a project's scheduled deployment jobs",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"}}})
        ),
        tool(
            "create_cron_job",
            "Schedule recurring deployments. schedule: 'every N minutes', 'every N hours', '*/N * * * *', or minutes",
            json!({"type": "object", "required": ["project_id", "name", "schedule"],
                   "properties": {"project_id": {"type": "string"},
                                  "name": {"type": "string"},
                                  "schedule": {"type": "string"},
                                  "branch": {"type": "string"},
                                  "environment_id": {"type": "string"}}})
        ),
        tool(
            "set_cron_job",
            "Enable or disable a scheduled deployment job",
            json!({"type": "object", "required": ["project_id", "job_id", "enabled"],
                   "properties": {"project_id": {"type": "string"},
                                  "job_id": {"type": "string"},
                                  "enabled": {"type": "boolean"}}})
        ),
        tool(
            "update_project_config",
            "Shallow-merge keys into a project's config (deployment_rules incl. preview policy, build/start/output settings). Pass a partial config object.",
            json!({"type": "object", "required": ["project_id", "config"],
                   "properties": {"project_id": {"type": "string"},
                                  "config": {"type": "object"}}})
        ),
        tool(
            "create_deploy_hook",
            "Mint a tokenized deploy-hook URL for the project (POST it to trigger a deploy without auth). Token returned once.",
            json!({"type": "object", "required": ["project_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "name": {"type": "string"},
                                  "branch": {"type": "string"}}})
        ),
        tool(
            "delete_deploy_hook",
            "Revoke a deploy hook by id",
            json!({"type": "object", "required": ["project_id", "hook_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "hook_id": {"type": "string"}}})
        ),
        tool(
            "delete_cron_job",
            "Remove a scheduled deployment job",
            json!({"type": "object", "required": ["project_id", "job_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "job_id": {"type": "string"}}})
        ),
        tool(
            "add_domain",
            "Add a domain/hostname to a project. Starts pending; run verify_domain after DNS points at the instance.",
            json!({"type": "object", "required": ["project_id", "hostname"],
                   "properties": {"project_id": {"type": "string"},
                                  "hostname": {"type": "string"},
                                  "type": {"type": "string", "enum": ["route","301","302","307","308"]},
                                  "environment_id": {"type": "string"}}})
        ),
        tool(
            "verify_domain",
            "Mark a project domain active (manual flow until DNS verification lands)",
            json!({"type": "object", "required": ["project_id", "domain_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "domain_id": {"type": "integer"}}})
        ),
        tool(
            "delete_domain",
            "Remove a domain from a project and rewrite routing. Cloudflare record cleanup is skipped — report shows the hostname for manual cleanup.",
            json!({"type": "object", "required": ["project_id", "domain_id"],
                   "properties": {"project_id": {"type": "string"},
                                  "domain_id": {"type": "integer"}}})
        ),
        tool(
            "list_storage",
            "List storage resources across the caller's teams (optionally scoped to team_id)",
            json!({"type": "object",
                   "properties": {"team_id": {"type": "string"}}})
        ),
        tool(
            "create_storage",
            "Provision managed storage and enqueue its provisioning job. type: database|volume|kv|queue; engine: sqlite|postgres|mongodb|redis (defaults per type).",
            json!({"type": "object", "required": ["team_id", "name", "type"],
                   "properties": {"team_id": {"type": "string"},
                                  "name": {"type": "string"},
                                  "type": {"type": "string", "enum": ["database","volume","kv","queue"]},
                                  "engine": {"type": "string"}}})
        ),
        tool(
            "link_storage",
            "Link managed storage to a project (same team). Optional environment_ids scopes the link.",
            json!({"type": "object", "required": ["team_id", "storage_id", "project_id"],
                   "properties": {"team_id": {"type": "string"},
                                  "storage_id": {"type": "string"},
                                  "project_id": {"type": "string"},
                                  "environment_ids": {"type": "array", "items": {"type": "string"}}}})
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
                Some(nid) => {
                    match runway_core::docker::node_client(&state.db, nid, &state.crypto).await {
                        Some(c) => c,
                        None => return Ok(json!({ "running": false })),
                    }
                }
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
        "get_project_usage" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let days = args["days"].as_i64().unwrap_or(30);
            crate::routes::projects::project_usage(state, &p.id, days).await
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
            let mut replayed = false;
            let dep = if has_remote {
                let (d, r) = crate::routes::deployments::trigger_deployment(
                    state,
                    &p,
                    &branch,
                    "api",
                    Some(user_id),
                    args["idempotency_key"].as_str(),
                )
                .await?;
                replayed = r;
                d
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
            Ok(
                json!({ "deployment_id": dep.id, "status": dep.status, "branch": dep.branch,
                       "idempotent_replay": replayed }),
            )
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
        "skip_deployment" => {
            let (d, _p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            runway_core::deploy::skip(&state.db, &state.bus, &d)
                .await
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(json!({ "deployment_id": d.id, "conclusion": "skipped" }))
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
        "promote_deployment" => {
            let (d, p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let env = args["environment_id"].as_str().unwrap_or("prod");
            let alias =
                runway_core::deploy::promote(&state.db, &state.bus, &state.settings, &p, env, &d)
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
            Ok(json!({ "environment_id": env, "deployment_id": alias.deployment_id }))
        }
        "promotion_diff" => {
            let (d, p) = deployment_for(state, user_id, arg(args, "deployment_id")?).await?;
            let env = args["environment_id"].as_str().unwrap_or("prod");
            let diff = runway_core::promote_diff::preview(
                &state.db,
                &state.crypto,
                &state.settings,
                &p,
                &d,
                env,
            )
            .await?;
            Ok(serde_json::to_value(diff).map_err(ApiError::internal)?)
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
        "update_project_config" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let patch = args["config"]
                .as_object()
                .ok_or_else(|| ApiError::bad_request("config must be an object"))?;
            let mut config = p.config.clone();
            let cfg = config.as_object_mut().expect("config is object");
            for (k, v) in patch {
                cfg.insert(k.clone(), v.clone());
            }
            sqlx::query("UPDATE project SET config = $1, updated_at = now() WHERE id = $2")
                .bind(&config)
                .bind(&p.id)
                .execute(&state.db)
                .await?;
            Ok(json!({ "ok": true, "config": config }))
        }
        "create_deploy_hook" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let name = args["name"].as_str().unwrap_or("default");
            let token = format!("dh_{}", runway_core::slugify::token_hex(16));
            let mut hooks = p
                .config
                .get("deploy_hooks")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if hooks.len() >= 20 {
                return Err(ApiError::bad_request("deploy hook limit reached (20)"));
            }
            hooks.push(json!({
                "id": runway_core::slugify::token_hex(8),
                "name": name,
                "token_hash": runway_core::crypto::sha256_hex(&token),
                "branch": args["branch"].as_str(),
                "created_at": chrono::Utc::now().to_rfc3339(),
            }));
            sqlx::query(
                "UPDATE project SET config = jsonb_set(config, '{deploy_hooks}', $1) WHERE id = $2",
            )
            .bind(json!(hooks))
            .bind(&p.id)
            .execute(&state.db)
            .await?;
            Ok(json!({
                "url": format!("{}://{}/api/deploy-hook/{}",
                    state.settings.url_scheme, state.settings.app_hostname, token),
            }))
        }
        "delete_deploy_hook" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let hook_id = arg(args, "hook_id")?;
            let hooks = p
                .config
                .get("deploy_hooks")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let kept: Vec<Value> = hooks
                .iter()
                .filter(|h| h["id"].as_str() != Some(hook_id))
                .cloned()
                .collect();
            if kept.len() == hooks.len() {
                return Err(ApiError::not_found("deploy hook"));
            }
            sqlx::query(
                "UPDATE project SET config = jsonb_set(config, '{deploy_hooks}', $1) WHERE id = $2",
            )
            .bind(json!(kept))
            .bind(&p.id)
            .execute(&state.db)
            .await?;
            Ok(json!({ "ok": true }))
        }
        "list_audit" => {
            let team = args["team_id"].as_str().filter(|s| !s.is_empty());
            let action = args["action"].as_str();
            let limit = args["limit"].as_i64();
            let before = args["before_id"].as_i64();
            let at = args["before_at"].as_str();
            let (entries, has_more) = crate::routes::notifications::visible_audit(
                state, user_id, team, action, limit, before, at,
            )
            .await?;
            Ok(json!({ "entries": entries, "has_more": has_more }))
        }
        "export_audit" => {
            let team = args["team_id"].as_str().filter(|s| !s.is_empty());
            let action = args["action"].as_str();
            let limit = args["limit"].as_i64();
            let (entries, has_more) = crate::routes::notifications::visible_audit_export(
                state, user_id, team, action, limit,
            )
            .await?;
            Ok(json!({ "entries": entries, "has_more": has_more }))
        }
        "list_cron_jobs" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let rows: Vec<crate::routes::projects::CronRow> = sqlx::query_as(
                "SELECT id, name, schedule, branch, environment_id, enabled,
                        last_run_at, next_run_at
                 FROM cron_job WHERE project_id = $1 ORDER BY created_at DESC",
            )
            .bind(&p.id)
            .fetch_all(&state.db)
            .await?;
            let jobs: Vec<Value> = rows
                .iter()
                .map(crate::routes::projects::cron_json)
                .collect();
            Ok(json!({ "cron_jobs": jobs }))
        }
        "create_cron_job" => {
            use crate::routes::projects::accessible_project_writer;
            let project_id = arg(args, "project_id")?.to_string();
            let project = accessible_project_writer(state, user_id, &project_id).await?;
            let name = args["name"].as_str().unwrap_or("").trim();
            let schedule = args["schedule"].as_str().unwrap_or("").trim();
            if name.is_empty() || schedule.is_empty() {
                return Err(ApiError::bad_request("name and schedule are required"));
            }
            let interval = runway_core::cron::parse_schedule(schedule);
            if interval == 0 {
                return Err(ApiError::bad_request(
                    "invalid schedule — use 'every N minutes', 'every N hours', '*/N * * * *', or minutes",
                ));
            }
            let branch = args["branch"].as_str().unwrap_or("main").to_string();
            let environment_id = args["environment_id"].as_str().map(str::to_string);
            let next_run = chrono::Utc::now() + chrono::Duration::minutes(interval as i64);
            let jid = runway_core::slugify::token_hex(16);
            let row: crate::routes::projects::CronRow = sqlx::query_as(
                "INSERT INTO cron_job (id, project_id, name, schedule, branch,
                                       environment_id, next_run_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)
                 RETURNING id, name, schedule, branch, environment_id, enabled,
                           last_run_at, next_run_at",
            )
            .bind(&jid)
            .bind(&project.id)
            .bind(name)
            .bind(schedule)
            .bind(&branch)
            .bind(&environment_id)
            .bind(next_run)
            .fetch_one(&state.db)
            .await?;
            Ok(crate::routes::projects::cron_json(&row))
        }
        "set_cron_job" => {
            use crate::routes::projects::accessible_project_writer;
            let project_id = arg(args, "project_id")?.to_string();
            let project = accessible_project_writer(state, user_id, &project_id).await?;
            let Some(enabled) = args["enabled"].as_bool() else {
                return Err(ApiError::bad_request("enabled (boolean) is required"));
            };
            let job_id = arg(args, "job_id")?;
            let row: Option<crate::routes::projects::CronRow> = sqlx::query_as(
                "UPDATE cron_job SET enabled = $3,
                    next_run_at = CASE WHEN $3 AND (next_run_at IS NULL OR next_run_at < now())
                        THEN now() ELSE next_run_at END,
                    updated_at = now()
                 WHERE id = $1 AND project_id = $2
                 RETURNING id, name, schedule, branch, environment_id, enabled,
                           last_run_at, next_run_at",
            )
            .bind(job_id)
            .bind(&project.id)
            .bind(enabled)
            .fetch_optional(&state.db)
            .await?;
            row.map(|r| crate::routes::projects::cron_json(&r))
                .ok_or_else(|| ApiError::not_found("cron job"))
        }
        "delete_cron_job" => {
            use crate::routes::projects::accessible_project_writer;
            let project_id = arg(args, "project_id")?.to_string();
            let project = accessible_project_writer(state, user_id, &project_id).await?;
            let res = sqlx::query("DELETE FROM cron_job WHERE id = $1 AND project_id = $2")
                .bind(arg(args, "job_id")?)
                .bind(&project.id)
                .execute(&state.db)
                .await?;
            if res.rows_affected() == 0 {
                return Err(ApiError::not_found("cron job"));
            }
            Ok(json!({ "ok": true }))
        }
        "add_domain" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let hostname = arg(args, "hostname")?.trim().to_lowercase();
            if hostname.is_empty()
                || hostname.len() > 253
                || !hostname
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            {
                return Err(ApiError::bad_request("invalid hostname"));
            }
            let ty = args["type"].as_str().unwrap_or("route");
            if !["route", "301", "302", "307", "308"].contains(&ty) {
                return Err(ApiError::bad_request("invalid domain type"));
            }
            let domain: runway_core::models::Domain = sqlx::query_as(
                "INSERT INTO domain (project_id, hostname, type, environment_id, status)
                 VALUES ($1,$2,$3,$4,'pending') RETURNING *",
            )
            .bind(&p.id)
            .bind(&hostname)
            .bind(ty)
            .bind(args["environment_id"].as_str())
            .fetch_one(&state.db)
            .await?;
            Ok(json!(domain))
        }
        "verify_domain" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let domain_id = args["domain_id"]
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("domain_id must be an integer"))?;
            let res = sqlx::query(
                "UPDATE domain SET status = 'active', last_checked_at = now()
                 WHERE id = $1 AND project_id = $2",
            )
            .bind(domain_id)
            .bind(&p.id)
            .execute(&state.db)
            .await?;
            if res.rows_affected() == 0 {
                return Err(ApiError::not_found("domain"));
            }
            runway_core::traefik::update_project_config(&state.db, &p, &state.settings, &[])
                .await?;
            Ok(json!({ "ok": true }))
        }
        "delete_domain" => {
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            let domain_id = args["domain_id"]
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("domain_id must be an integer"))?;
            let row: Option<(String,)> =
                sqlx::query_as("SELECT hostname FROM domain WHERE id = $1 AND project_id = $2")
                    .bind(domain_id)
                    .bind(&p.id)
                    .fetch_optional(&state.db)
                    .await?;
            let Some((hostname,)) = row else {
                return Err(ApiError::not_found("domain"));
            };
            sqlx::query("DELETE FROM domain WHERE id = $1 AND project_id = $2")
                .bind(domain_id)
                .bind(&p.id)
                .execute(&state.db)
                .await?;
            runway_core::traefik::update_project_config(&state.db, &p, &state.settings, &[])
                .await?;
            Ok(json!({ "ok": true, "hostname": hostname,
                       "note": "cloudflare record cleanup skipped — remove the DNS record manually" }))
        }
        "list_storage" => {
            let items: Vec<runway_core::models::Storage> = match args["team_id"].as_str() {
                Some(tid) => {
                    sqlx::query_as(
                        "SELECT s.* FROM storage s
                     JOIN team_member tm ON tm.team_id = s.team_id
                     WHERE tm.user_id = $1 AND s.team_id = $2",
                    )
                    .bind(user_id)
                    .bind(tid)
                    .fetch_all(&state.db)
                    .await?
                }
                None => {
                    sqlx::query_as(
                        "SELECT s.* FROM storage s
                     JOIN team_member tm ON tm.team_id = s.team_id
                     WHERE tm.user_id = $1",
                    )
                    .bind(user_id)
                    .fetch_all(&state.db)
                    .await?
                }
            };
            Ok(json!(items))
        }
        "create_storage" => {
            let team_id = arg(args, "team_id")?;
            let (tid, role) = crate::routes::cloudflare::team_role(state, user_id, team_id).await?;
            crate::routes::cloudflare::require_admin(&role)?;
            let name = arg(args, "name")?.trim();
            if name.is_empty()
                || name.len() > 100
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(ApiError::bad_request("name must be 1-100 chars [a-z0-9-_]"));
            }
            let ty = arg(args, "type")?;
            let engine = args["engine"]
                .as_str()
                .unwrap_or(if ty == "kv" { "redis" } else { "sqlite" })
                .to_string();
            let ok_engine = match ty {
                "database" => matches!(engine.as_str(), "sqlite" | "postgres" | "mongodb"),
                "kv" => engine == "redis",
                _ => true,
            };
            if !ok_engine {
                return Err(ApiError::bad_request("unsupported engine for type"));
            }
            let id = runway_core::slugify::token_hex(16);
            let res = sqlx::query(
                "INSERT INTO storage (id, name, type, status, config, team_id, created_by_user_id)
                 VALUES ($1,$2,$3,'pending',$4,$5,$6)",
            )
            .bind(&id)
            .bind(name)
            .bind(ty)
            .bind(json!({ "engine": engine }))
            .bind(&tid)
            .bind(user_id)
            .execute(&state.db)
            .await;
            if let Err(e) = res {
                if e.to_string().contains("ix_storage_team_name_lower")
                    || e.to_string().contains("storage_team_id_name_key")
                {
                    return Err(ApiError::bad_request("storage name already exists"));
                }
                return Err(e.into());
            }
            runway_core::deploy::enqueue(
                &state.db,
                "provision_storage",
                json!({ "storage_id": id }),
                0,
            )
            .await?;
            Ok(json!({ "storage_id": id, "status": "pending", "engine": engine }))
        }
        "link_storage" => {
            let team_id = arg(args, "team_id")?;
            let (tid, role) = crate::routes::cloudflare::team_role(state, user_id, team_id).await?;
            crate::routes::cloudflare::require_admin(&role)?;
            let storage_id = arg(args, "storage_id")?;
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT id FROM storage WHERE id = $1 AND team_id = $2")
                    .bind(storage_id)
                    .bind(&tid)
                    .fetch_optional(&state.db)
                    .await?;
            if exists.is_none() {
                return Err(ApiError::not_found("storage"));
            }
            let p = project_for(state, user_id, arg(args, "project_id")?).await?;
            if p.team_id != tid {
                return Err(ApiError::bad_request("project belongs to another team"));
            }
            let env_ids = args["environment_ids"].as_array().map(|v| json!(v));
            sqlx::query(
                "INSERT INTO storage_project (id, storage_id, project_id, environment_ids)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT (storage_id, project_id) DO UPDATE SET
                   environment_ids = EXCLUDED.environment_ids, updated_at = now()",
            )
            .bind(runway_core::slugify::token_hex(16))
            .bind(storage_id)
            .bind(&p.id)
            .bind(&env_ids)
            .execute(&state.db)
            .await?;
            Ok(json!({ "ok": true }))
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
