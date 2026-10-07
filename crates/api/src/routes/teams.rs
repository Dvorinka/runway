//! Team routes: list/get for the authenticated user, plus team-level
//! webhook CRUD (port of devpush team webhooks — events fan out to every
//! project or a `project_ids` subset).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::models::TeamWebhook;
use runway_core::webhook::WEBHOOK_EVENTS;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

async fn accessible_team(state: &AppState, user_id: i64, id: &str) -> ApiResult<String> {
    let team: Option<(String,)> = sqlx::query_as(
        "SELECT t.id FROM team t
         JOIN team_member tm ON tm.team_id = t.id
         WHERE tm.user_id = $1 AND t.id = $2 AND t.status != 'deleted'",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    team.map(|(id,)| id)
        .ok_or_else(|| ApiError::not_found("team"))
}

pub async fn list(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let teams: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT t.id, t.name, t.slug, tm.role FROM team t
         JOIN team_member tm ON tm.team_id = t.id
         WHERE tm.user_id = $1 AND t.status != 'deleted' ORDER BY t.name",
    )
    .bind(user.user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "teams": teams.iter().map(|(id, name, slug, role)| json!({
            "id": id, "name": name, "slug": slug, "role": role,
        })).collect::<Vec<_>>()
    }))
    .into_response())
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let team: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, slug FROM team WHERE id = $1")
            .bind(&team_id)
            .fetch_optional(&state.db)
            .await?;
    let members: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, tm.role FROM team_member tm
         JOIN \"user\" u ON u.id = tm.user_id WHERE tm.team_id = $1",
    )
    .bind(&team_id)
    .fetch_all(&state.db)
    .await?;
    let Some((id, name, slug)) = team else {
        return Err(ApiError::not_found("team"));
    };
    Ok(Json(json!({
        "team": {
            "id": id, "name": name, "slug": slug,
            "members": members.iter().map(|(uid, uname, role)| json!({
                "user_id": uid, "username": uname, "role": role,
            })).collect::<Vec<_>>(),
        }
    }))
    .into_response())
}

// -- Team webhooks -----------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateTeamWebhook {
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    /// deployment.* event names; empty = all.
    pub events: Option<Vec<String>>,
    /// Restrict delivery to these project ids; None = all team projects.
    pub project_ids: Option<Vec<String>>,
}

fn webhook_json(w: &TeamWebhook) -> Value {
    json!({
        "id": w.id,
        "team_id": w.team_id,
        "name": w.name,
        "url": w.url,
        "has_secret": w.secret.is_some(),
        "events": w.events,
        "project_ids": w.project_ids,
        "status": w.status,
        "created_at": w.created_at,
    })
}

pub async fn list_webhooks(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let hooks: Vec<TeamWebhook> =
        sqlx::query_as("SELECT * FROM team_webhook WHERE team_id = $1 ORDER BY created_at")
            .bind(&team_id)
            .fetch_all(&state.db)
            .await?;
    Ok(
        Json(json!({ "webhooks": hooks.iter().map(webhook_json).collect::<Vec<_>>() }))
            .into_response(),
    )
}

pub async fn create_webhook(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateTeamWebhook>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let events = body.events.unwrap_or_default();
    if body.name.trim().is_empty() || body.name.len() > 100 {
        return Err(ApiError::bad_request("invalid webhook name"));
    }
    if !(body.url.starts_with("http://") || body.url.starts_with("https://"))
        || body.url.len() > 2048
    {
        return Err(ApiError::bad_request("webhook url must be http(s)"));
    }
    for e in &events {
        if !WEBHOOK_EVENTS.contains(&e.as_str()) {
            return Err(ApiError::bad_request(format!("unknown event '{e}'")));
        }
    }
    let secret_enc = match &body.secret {
        Some(s) if !s.is_empty() => Some(state.crypto.encrypt(s).map_err(ApiError::internal)?),
        _ => None,
    };
    let wid = runway_core::slugify::token_hex(16);
    sqlx::query(
        "INSERT INTO team_webhook (id, team_id, name, url, secret, events, project_ids, created_by_user_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(&wid)
    .bind(&team_id)
    .bind(body.name.trim())
    .bind(&body.url)
    .bind(&secret_enc)
    .bind(json!(events))
    .bind(body.project_ids.as_ref().map(|ids| json!(ids)))
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    let hook: TeamWebhook = sqlx::query_as("SELECT * FROM team_webhook WHERE id = $1")
        .bind(&wid)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(webhook_json(&hook))).into_response())
}

pub async fn delete_webhook(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, webhook_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let res = sqlx::query("DELETE FROM team_webhook WHERE id = $1 AND team_id = $2")
        .bind(&webhook_id)
        .bind(&team_id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("webhook"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
