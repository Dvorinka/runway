//! `/api` surface: API keys, deploy-token deploy endpoint.
//! The deploy endpoint is the CI/git-agnostic trigger (`dp_` token).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::deploy;
use runway_core::models::ApiKey;
use runway_core::slugify::token_hex;

use crate::auth::{verify_deploy_token, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::deployments::trigger_deployment;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateKey {
    pub name: String,
}

pub async fn create_key(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateKey>,
) -> ApiResult<Response> {
    let (raw, hash) = ApiKey::generate();
    let id = token_hex(16);
    sqlx::query("INSERT INTO api_key (id, user_id, name, token) VALUES ($1,$2,$3,$4)")
        .bind(&id)
        .bind(user.user.id)
        .bind(&body.name)
        .bind(&hash)
        .execute(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "token": raw }))).into_response())
}

pub async fn list_keys(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let keys: Vec<ApiKey> =
        sqlx::query_as("SELECT * FROM api_key WHERE user_id = $1 ORDER BY created_at DESC")
            .bind(user.user.id)
            .fetch_all(&state.db)
            .await?;
    let items: Vec<_> = keys
        .iter()
        .map(|k| {
            json!({
                "id": k.id,
                "name": k.name,
                "status": k.status,
                "last_used_at": k.last_used_at,
                "created_at": k.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "keys": items })).into_response())
}

pub async fn revoke_key(
    user: AuthUser,
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<Response> {
    sqlx::query("UPDATE api_key SET status = 'revoked' WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// POST /api/deploy — deploy-token trigger (CI).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct DeployRequest {
    /// Defaults to the token's project.
    pub project_id: Option<String>,
    /// Defaults to the production branch.
    pub branch: Option<String>,
}

pub async fn api_deploy(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<DeployRequest>>,
) -> ApiResult<Response> {
    let token = headers
        .get("X-Deploy-Token")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer ").map(String::from))
                .filter(|t| t.starts_with("rw_") || t.starts_with("dp_"))
        })
        .ok_or_else(|| ApiError::unauthorized("deploy token required"))?;
    let token = verify_deploy_token(&state, &token).await?;

    let body = body.map(|b| b.0);
    let project_id = body
        .as_ref()
        .and_then(|b| b.project_id.clone())
        .unwrap_or_else(|| token.project_id.clone());
    if project_id != token.project_id {
        return Err(ApiError::forbidden("token not scoped to this project"));
    }
    let project = deploy::get_project(&state.db, &project_id)
        .await?
        .ok_or_else(|| ApiError::not_found("project"))?;
    let branch = body
        .and_then(|b| b.branch)
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| {
            if project.repo_branch.is_empty() {
                "main".into()
            } else {
                project.repo_branch.clone()
            }
        });

    let env = project
        .environment_for_branch(&branch)
        .ok_or_else(|| ApiError::bad_request("no environment matches branch"))?;
    if !token.can_deploy_environment(&env.id) {
        return Err(ApiError::forbidden("token cannot deploy this environment"));
    }

    let dep = trigger_deployment(&state, &project, &branch, "api", None).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": dep.id,
            "status": dep.status,
            "url": dep.url(project.slug.as_deref().unwrap_or(&project.id), &state.settings),
        })),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// OpenAPI
// ---------------------------------------------------------------------------

/// `GET /api/v1/openapi.json` — machine-readable API surface for agents.
/// jarvis: hand-maintained spec; upgrade to utoipa if drift becomes painful.
pub async fn openapi() -> Response {
    // (path, [(method, summary)])
    const ROUTES: &[(&str, &[(&str, &str)])] = &[
        ("/health", &[("get", "Liveness probe")]),
        ("/api/auth/login", &[("post", "Email + password sign-in {email, password}")]),
        ("/api/auth/register", &[("post", "Create account {email, password, username?} — allowlist-gated")]),
        ("/api/auth/me", &[("get", "Current user")]),
        ("/api/auth/logout", &[("post", "Destroy session")]),
        ("/api/github/webhook", &[("post", "GitHub App webhook (push + pull_request)")]),
        ("/api/v1/github/app/status", &[("get", "GitHub App registration status")]),
        ("/api/v1/github/app/register", &[("get", "Start GitHub App manifest registration (admin)")]),
        ("/api/v1/github/app/callback", &[("get", "GitHub App manifest callback (admin)")]),
        ("/api/v1/github/installations", &[("get", "List GitHub App installations")]),
        ("/api/v1/github/installations/{id}/repos", &[("get", "List repos for an installation")]),
        ("/api/v1/projects", &[
            ("get", "List accessible projects"),
            ("post", "Create project {name, repo_id, repo_full_name, installation_id, branch?, preset?, config?} — preset auto-detected when omitted"),
        ]),
        ("/api/v1/projects/{id}", &[
            ("get", "Project details"),
            ("patch", "Update name/branch/config/environments"),
            ("delete", "Delete project"),
        ]),
        ("/api/v1/projects/{id}/env", &[
            ("get", "List env vars (values masked)"),
            ("put", "Replace all env vars [{key, value, environment?}]"),
            ("patch", "Upsert/delete individual vars [{key, value?, environment?, delete?}]"),
        ]),
        ("/api/v1/projects/{id}/deploy-tokens", &[("post", "Mint rw_ deploy token {name, environment_id?}")]),
        ("/api/v1/projects/{id}/domains", &[
            ("get", "List custom domains"),
            ("post", "Add domain {hostname, type?, environment_id?}"),
        ]),
        ("/api/v1/projects/{id}/domains/{domain_id}", &[("delete", "Remove domain (cleans CF DNS + tunnel ingress)")]),
        ("/api/v1/projects/{id}/domains/{domain_id}/assign-cloudflare", &[("post", "One-click Cloudflare DNS + tunnel ingress")]),
        ("/api/v1/projects/{id}/webhooks", &[
            ("get", "List project webhooks"),
            ("post", "Create webhook {name, url, secret?, events?}"),
        ]),
        ("/api/v1/projects/{id}/webhooks/{webhook_id}", &[("delete", "Delete webhook")]),
        ("/api/v1/projects/{id}/deployments", &[
            ("get", "List deployments (?limit)"),
            ("post", "Deploy latest commit {branch?}"),
        ]),
        ("/api/v1/projects/{id}/deployments/upload", &[("post", "Deploy a local tarball (application/gzip body, ≤512MB, no git)")]),
        ("/api/v1/projects/{id}/environments/{env_id}/rollback", &[("post", "Roll back an environment to its previous deployment")]),
        ("/api/v1/projects/{id}/events", &[("get", "SSE stream of project deployment events")]),
        ("/api/v1/deployments/{id}", &[("get", "Deployment details incl. status/conclusion/urls")]),
        ("/api/v1/deployments/{id}/cancel", &[("post", "Cancel a running deployment")]),
        ("/api/v1/deployments/{id}/skip", &[("post", "Skip a queued deployment")]),
        ("/api/v1/deployments/{id}/redeploy", &[("post", "New deployment, same commit")]),
        ("/api/v1/deployments/{id}/logs", &[("get", "Plain-text log tail (?tail=N)")]),
        ("/api/v1/deployments/{id}/logs/stream", &[("get", "SSE live log stream")]),
        ("/api/v1/teams", &[("get", "List teams for the current user")]),
        ("/api/v1/teams/{id}", &[("get", "Team details + members")]),
        ("/api/v1/teams/{id}/webhooks", &[
            ("get", "List team webhooks"),
            ("post", "Create team webhook {name, url, secret?, events?, project_ids?}"),
        ]),
        ("/api/v1/teams/{id}/webhooks/{webhook_id}", &[("delete", "Delete team webhook")]),
        ("/api/v1/keys", &[
            ("get", "List API keys"),
            ("post", "Create API key {name} — raw ak_ returned once"),
        ]),
        ("/api/v1/keys/{id}", &[("delete", "Revoke an API key")]),
        ("/api/deploy", &[("post", "Trigger deploy with rw_ token {project, branch?}")]),
        ("/api/mcp", &[("post", "MCP JSON-RPC 2.0 (initialize, tools/list, tools/call) — Bearer ak_ auth")]),
    ];

    let mut paths = serde_json::Map::new();
    for (path, methods) in ROUTES {
        let mut ops = serde_json::Map::new();
        for (method, summary) in *methods {
            ops.insert(
                (*method).into(),
                json!({ "summary": summary,
                        "responses": { "200": { "description": "OK" } } }),
            );
        }
        paths.insert((*path).into(), Value::Object(ops));
    }

    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Runway API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Self-hosted deployment platform. Auth: Bearer ak_ for /api/v1, rw_ deploy token for /api/deploy, GitHub App signature for webhooks.",
        },
        "servers": [{ "url": "/" }],
        "paths": Value::Object(paths),
    }))
    .into_response()
}
