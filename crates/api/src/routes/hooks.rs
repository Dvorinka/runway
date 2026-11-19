//! Deploy hooks — tokenized URLs that trigger a deployment without a
//! login (Vercel "Deploy Hooks" parity; CMS/CI/external triggers).
//!
//! Hooks live in `project.config.deploy_hooks` — no migration needed:
//! `[{id, name, token_hash, branch, created_at}]`. The token itself is
//! returned once at creation and only its sha256 is stored.
//!
//! `POST /api/deploy-hook/{token}` is unauthenticated by design; the
//! 128-bit token is the credential. It is rate-unlimited like any
//! inbound webhook — treat it as a secret.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::crypto::sha256_hex;
use runway_core::deploy::{self, CommitInfo};
use runway_core::slugify::token_hex;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn hooks_of(project: &runway_core::models::Project) -> Vec<Value> {
    project
        .config
        .get("deploy_hooks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

async fn save_hooks(state: &AppState, project_id: &str, hooks: &[Value]) -> ApiResult<()> {
    sqlx::query(
        "UPDATE project SET config = jsonb_set(config, '{deploy_hooks}', $1) WHERE id = $2",
    )
    .bind(json!(hooks))
    .bind(project_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// `GET /api/v1/projects/{id}/hooks` — list hooks (token hashes masked).
pub async fn list(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let project =
        crate::routes::projects::accessible_project_writer(&state, user.user.id, &project_id)
            .await?;
    let hooks: Vec<Value> = hooks_of(&project)
        .into_iter()
        .map(|h| {
            json!({
                "id": h["id"], "name": h["name"], "branch": h["branch"],
                "created_at": h["created_at"],
                "url": format!("/api/deploy-hook/…{}", &h["token_hash"].as_str().unwrap_or("")[..6]),
            })
        })
        .collect();
    Ok(Json(json!({ "hooks": hooks })).into_response())
}

#[derive(Deserialize)]
pub struct CreateHook {
    name: Option<String>,
    branch: Option<String>,
}

/// `POST /api/v1/projects/{id}/hooks` — mint a hook. The token is
/// returned once in `url`; store it — only the hash persists.
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateHook>,
) -> ApiResult<Response> {
    let mut project =
        crate::routes::projects::accessible_project_writer(&state, user.user.id, &project_id)
            .await?;
    let name = body.name.unwrap_or_else(|| "default".into());
    if name.trim().is_empty() || name.len() > 64 {
        return Err(ApiError::bad_request("hook name must be 1–64 chars"));
    }
    let token = format!("dh_{}", token_hex(16));
    let hook = json!({
        "id": token_hex(8),
        "name": name,
        "token_hash": sha256_hex(&token),
        "branch": body.branch,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    let mut hooks = hooks_of(&project);
    if hooks.len() >= 20 {
        return Err(ApiError::bad_request("deploy hook limit reached (20)"));
    }
    hooks.push(hook.clone());
    save_hooks(&state, &project.id, &hooks).await?;
    project.config["deploy_hooks"] = json!(hooks);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "hook": { "id": hook["id"], "name": hook["name"], "branch": hook["branch"] },
            "url": format!(
                "{}://{}/api/deploy-hook/{}",
                state.settings.url_scheme, state.settings.app_hostname, token
            ),
        })),
    )
        .into_response())
}

/// `DELETE /api/v1/projects/{id}/hooks/{hook_id}`
pub async fn delete(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, hook_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project =
        crate::routes::projects::accessible_project_writer(&state, user.user.id, &project_id)
            .await?;
    let hooks = hooks_of(&project);
    let kept: Vec<Value> = hooks
        .iter()
        .filter(|h| h["id"].as_str() != Some(hook_id.as_str()))
        .cloned()
        .collect();
    if kept.len() == hooks.len() {
        return Err(ApiError::not_found("deploy hook"));
    }
    save_hooks(&state, &project.id, &kept).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /api/deploy-hook/{token}` — public trigger. Empty body or
/// `{"ref": "branch"}`; the hook's stored branch is the default.
pub async fn trigger(
    State(state): State<AppState>,
    Path(token): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Response> {
    // Constant-work lookup: hash first, then compare candidate rows.
    let hash = sha256_hex(&token);
    let projects: Vec<runway_core::models::Project> = sqlx::query_as(
        "SELECT * FROM project WHERE config ? 'deploy_hooks' AND status != 'deleted'",
    )
    .fetch_all(&state.db)
    .await?;
    let (project, hook) = projects
        .into_iter()
        .filter_map(|p| {
            hooks_of(&p)
                .into_iter()
                .find(|h| h["token_hash"].as_str() == Some(hash.as_str()))
                .map(|h| (p, h))
        })
        .next()
        .ok_or_else(|| ApiError::not_found("deploy hook"))?;

    let body = body.map(|j| j.0).unwrap_or(json!({}));
    let branch = body
        .get("ref")
        .and_then(|v| v.as_str())
        .or_else(|| hook["branch"].as_str())
        .map(str::to_string)
        .or_else(|| {
            project
                .active_environments()
                .first()
                .map(|e| e.branch.clone())
        })
        .ok_or_else(|| ApiError::bad_request("no branch to deploy"))?;

    let commit = CommitInfo {
        sha: String::new(), // resolved to the real head by the worker
        author: "deploy-hook".into(),
        message: format!("Deploy hook: {}", hook["name"].as_str().unwrap_or("")),
        timestamp: None,
    };
    let dep = deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        &project,
        &branch,
        &commit,
        "deploy_hook",
        None,
        None,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "deployment_id": dep.id, "branch": branch })),
    )
        .into_response())
}
