//! Avatar upload/delete/serve for users, teams, and projects.
//! Raw image body (`Content-Type: image/*`) like the upload-deploy
//! endpoint — no multipart needed for a single file.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use runway_core::avatars;

use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::state::AppState;

/// `GET /api/avatars/{kind}/{id}` — served to any viewer (avatars render
/// in shared pages), 404 when unset. Short cache so uploads propagate.
pub async fn get(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> Response {
    if !matches!(kind.as_str(), "user" | "team" | "project") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match avatars::find(std::path::Path::new(&state.settings.data_dir), &kind, &id).await {
        Some((path, mime)) => match tokio::fs::read(&path).await {
            Ok(bytes) => (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "public, max-age=60"),
                ],
                bytes,
            )
                .into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Shared upload/delete core. `set_flag` persists has_avatar on the row.
async fn put(
    state: &AppState,
    kind: &str,
    id: &str,
    headers: &HeaderMap,
    body: &Bytes,
    table: &str,
) -> ApiResult<Response> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    avatars::save(
        std::path::Path::new(&state.settings.data_dir),
        kind,
        id,
        content_type,
        body,
    )
    .await?;
    sqlx::query(&format!(
        "UPDATE {table} SET has_avatar = TRUE, updated_at = now() WHERE id::text = $1"
    ))
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "has_avatar": true })).into_response())
}

async fn delete(state: &AppState, kind: &str, id: &str, table: &str) -> ApiResult<Response> {
    avatars::remove(std::path::Path::new(&state.settings.data_dir), kind, id).await?;
    sqlx::query(&format!(
        "UPDATE {table} SET has_avatar = FALSE, updated_at = now() WHERE id::text = $1"
    ))
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "has_avatar": false })).into_response())
}

/// `PUT /api/auth/avatar` — own profile picture.
pub async fn put_user(
    user: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    put(
        &state,
        "user",
        &user.user.id.to_string(),
        &headers,
        &body,
        "\"user\"",
    )
    .await
}

pub async fn delete_user(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    delete(&state, "user", &user.user.id.to_string(), "\"user\"").await
}

/// `PUT /api/v1/teams/{id}/avatar` — team admin+.
pub async fn put_team(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    put(&state, "team", &team_id, &headers, &body, "team").await
}

pub async fn delete_team(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    delete(&state, "team", &team_id, "team").await
}

/// `PUT /api/v1/projects/{id}/avatar` — project writer (creator/admin).
pub async fn put_project(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let project =
        crate::routes::projects::accessible_project_writer(&state, user.user.id, &id).await?;
    put(&state, "project", &project.id, &headers, &body, "project").await
}

pub async fn delete_project(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project =
        crate::routes::projects::accessible_project_writer(&state, user.user.id, &id).await?;
    delete(&state, "project", &project.id, "project").await
}
