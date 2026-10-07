//! `/api` surface: API keys, deploy-token deploy endpoint.
//! The deploy endpoint is the CI/git-agnostic trigger (`dp_` token).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

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
