//! Team-scoped storage — port of devpush `team.py` storage routes.
//! Mutations enqueue provision/deprovision/reset jobs; reads are direct.

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

/// Serialize a storage row; the encrypted password is never exposed,
/// only `has_password`.
fn storage_json(s: &runway_core::models::Storage) -> Value {
    let mut config = s.config.clone();
    if let Some(obj) = config.as_object_mut() {
        obj.remove("password_enc");
        obj.insert(
            "has_password".into(),
            s.config["password_enc"].is_string().into(),
        );
    }
    json!({
        "id": s.id, "name": s.name, "type": s.r#type,
        "status": s.status, "engine": s.engine(),
        "config": config, "error": s.error,
        "created_at": s.created_at,
    })
}

async fn get_storage(
    state: &AppState,
    team_id: &str,
    id: &str,
) -> ApiResult<runway_core::models::Storage> {
    sqlx::query_as("SELECT * FROM storage WHERE id = $1 AND team_id = $2")
        .bind(id)
        .bind(team_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("storage"))
}

pub async fn list(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> ApiResult<Response> {
    let (tid, _) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    let rows: Vec<runway_core::models::Storage> = sqlx::query_as(
        "SELECT * FROM storage WHERE team_id = $1 AND status != 'deleted'
         ORDER BY created_at",
    )
    .bind(&tid)
    .fetch_all(&state.db)
    .await?;
    // Linked projects per storage — lets the dashboard render link/unlink.
    let ids: Vec<String> = rows.iter().map(|s| s.id.clone()).collect();
    let links: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT sp.storage_id, sp.project_id, p.name FROM storage_project sp
         JOIN project p ON p.id = sp.project_id
         WHERE sp.storage_id = ANY($1) ORDER BY p.name",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let storage: Vec<Value> = rows
        .iter()
        .map(|s| {
            let mut v = storage_json(s);
            v["links"] = json!(links
                .iter()
                .filter(|(sid, _, _)| sid == &s.id)
                .map(|(_, pid, pname)| json!({ "project_id": pid, "project_name": pname }))
                .collect::<Vec<_>>());
            v
        })
        .collect();
    Ok(Json(json!({ "storage": storage })).into_response())
}

#[derive(Deserialize)]
pub struct CreateStorage {
    pub name: String,
    /// database | volume | kv | queue
    pub r#type: String,
    /// sqlite | postgres | mongodb (database); redis (kv)
    pub engine: Option<String>,
}

pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(body): Json<CreateStorage>,
) -> ApiResult<Response> {
    let (tid, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    let name = body.name.trim();
    if name.is_empty()
        || name.len() > 100
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request("name must be 1-100 chars [a-z0-9-_]"));
    }
    if !matches!(body.r#type.as_str(), "database" | "volume" | "kv" | "queue") {
        return Err(ApiError::bad_request(
            "type must be database|volume|kv|queue",
        ));
    }
    let engine = body.engine.unwrap_or_else(|| {
        match body.r#type.as_str() {
            "kv" => "redis",
            _ => "sqlite",
        }
        .into()
    });
    let ok_engine = match body.r#type.as_str() {
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
    .bind(&body.r#type)
    .bind(json!({ "engine": engine }))
    .bind(&tid)
    .bind(user.user.id)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        // Unique (team_id, lower(name)) violation → friendly error.
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
    .await
    .map_err(ApiError::from)?;
    let mut a = runway_core::audit::Audit::new("storage.create");
    a.user_id = Some(user.user.id);
    a.team_id = Some(tid);
    a.resource_type = Some("storage");
    a.resource_id = Some(id.clone());
    a.detail = Some(format!("type={} engine={engine}", body.r#type));
    runway_core::audit::log(&state.db, a).await;
    Ok(StatusCode::CREATED.into_response())
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path((team_id, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (tid, _) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    let storage = get_storage(&state, &tid, &id).await?;
    Ok(Json(json!({ "storage": storage_json(&storage) })).into_response())
}

pub async fn delete(
    user: AuthUser,
    State(state): State<AppState>,
    Path((team_id, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (tid, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    get_storage(&state, &tid, &id).await?;
    sqlx::query("UPDATE storage SET status = 'deleted', updated_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&state.db)
        .await?;
    runway_core::deploy::enqueue(
        &state.db,
        "deprovision_storage",
        json!({ "storage_id": id }),
        0,
    )
    .await
    .map_err(ApiError::from)?;
    let mut a = runway_core::audit::Audit::new("storage.delete");
    a.user_id = Some(user.user.id);
    a.team_id = Some(tid);
    a.resource_type = Some("storage");
    a.resource_id = Some(id);
    runway_core::audit::log(&state.db, a).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn reset(
    user: AuthUser,
    State(state): State<AppState>,
    Path((team_id, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (tid, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    get_storage(&state, &tid, &id).await?;
    sqlx::query("UPDATE storage SET status = 'resetting', updated_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&state.db)
        .await?;
    runway_core::deploy::enqueue(&state.db, "reset_storage", json!({ "storage_id": id }), 0)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(json!({ "ok": true })).into_response())
}

// -- Project links -----------------------------------------------------------

#[derive(Deserialize)]
pub struct LinkStorage {
    pub project_id: String,
    /// Restrict to these environment ids; null = all.
    pub environment_ids: Option<Vec<String>>,
}

/// Link a storage to a project — binds `/data/<type>/<name>` into the
/// deployment container and joins the storage network.
pub async fn link(
    user: AuthUser,
    State(state): State<AppState>,
    Path((team_id, id)): Path<(String, String)>,
    Json(body): Json<LinkStorage>,
) -> ApiResult<Response> {
    let (tid, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    get_storage(&state, &tid, &id).await?;
    // Project must belong to the same team.
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM project WHERE id = $1 AND team_id = $2")
            .bind(&body.project_id)
            .bind(&tid)
            .fetch_optional(&state.db)
            .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("project"));
    }
    let env_ids = body.environment_ids.map(|v| json!(v));
    sqlx::query(
        "INSERT INTO storage_project (id, storage_id, project_id, environment_ids)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (storage_id, project_id) DO UPDATE SET
           environment_ids = EXCLUDED.environment_ids, updated_at = now()",
    )
    .bind(runway_core::slugify::token_hex(16))
    .bind(&id)
    .bind(&body.project_id)
    .bind(&env_ids)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "ok": true })).into_response())
}

pub async fn unlink(
    user: AuthUser,
    State(state): State<AppState>,
    Path((team_id, id, project_id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let (tid, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &team_id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    get_storage(&state, &tid, &id).await?;
    sqlx::query("DELETE FROM storage_project WHERE storage_id = $1 AND project_id = $2")
        .bind(&id)
        .bind(&project_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
