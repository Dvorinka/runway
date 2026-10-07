//! Per-team Cloudflare connections + tunnels — port of devpush
//! `routers/cloudflare.py`. Token connect only for now; the OAuth dance
//! is deferred (needs CF OAuth client creds — jarvis: add when requested).
//!
//! - `GET    /api/v1/teams/{id}/cloudflare`          — connection + container status
//! - `POST   /api/v1/teams/{id}/cloudflare/connect`  — verify token, ensure tunnel
//! - `DELETE /api/v1/teams/{id}/cloudflare`          — stop container, delete tunnel
//! - `GET    /api/v1/teams/{id}/cloudflare/zones`    — zone picker data

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// Team membership with role — admin required for connect/disconnect,
/// member for reads (parity with devpush `get_access(role, "admin")`).
pub async fn team_role(
    state: &AppState,
    user_id: i64,
    team_id: &str,
) -> ApiResult<(String, String)> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT t.id, tm.role FROM team t
         JOIN team_member tm ON tm.team_id = t.id
         WHERE tm.user_id = $1 AND t.id = $2 AND t.status != 'deleted'",
    )
    .bind(user_id)
    .bind(team_id)
    .fetch_optional(&state.db)
    .await?;
    row.ok_or_else(|| ApiError::not_found("team"))
}

fn require_admin(role: &str) -> ApiResult<()> {
    if matches!(role, "owner" | "admin") {
        Ok(())
    } else {
        Err(ApiError::forbidden("team admin role required"))
    }
}

async fn connection(
    state: &AppState,
    team_id: &str,
) -> ApiResult<Option<runway_core::models::CloudflareConnection>> {
    sqlx::query_as("SELECT * FROM cloudflare_connection WHERE team_id = $1")
        .bind(team_id)
        .fetch_optional(&state.db)
        .await
        .map_err(ApiError::from)
}

/// Connection + live container status.
pub async fn status(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> ApiResult<Response> {
    team_role(&state, user.user.id, &team_id).await?;
    let Some(conn) = connection(&state, &team_id).await? else {
        return Ok(Json(json!({ "connected": false })).into_response());
    };
    let container = match &state.docker {
        Some(d) => runway_core::tunnel::cloudflared_status(d, &container_name(&team_id)).await,
        None => "unavailable".into(),
    };
    Ok(Json(json!({
        "connected": true,
        "account_id": conn.account_id,
        "account_name": conn.account_name,
        "auth_method": conn.auth_method,
        "tunnel_id": conn.tunnel_id,
        "tunnel_name": conn.tunnel_name,
        "container_status": container,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct ConnectBody {
    pub api_token: String,
    /// Optional explicit account; defaults to the token's first account
    /// (devpush `verify_token` behavior).
    pub account_id: Option<String>,
}

pub async fn connect(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(body): Json<ConnectBody>,
) -> ApiResult<Response> {
    let (_, role) = team_role(&state, user.user.id, &team_id).await?;
    require_admin(&role)?;

    let cf = runway_core::cloudflare::CloudflareClient::new(&body.api_token);
    cf.verify()
        .await
        .map_err(|_| ApiError::bad_request("cloudflare token verification failed"))?;
    let accounts = cf.list_accounts().await.map_err(ApiError::from)?;
    let account = match &body.account_id {
        Some(id) => accounts
            .iter()
            .find(|a| a["id"].as_str() == Some(id))
            .cloned()
            .ok_or_else(|| ApiError::bad_request("account_id not accessible by token"))?,
        None => accounts
            .first()
            .cloned()
            .ok_or_else(|| ApiError::bad_request("token has no accessible accounts"))?,
    };
    let account_id = account["id"].as_str().unwrap_or_default().to_string();
    let account_name = account["name"].as_str().unwrap_or_default().to_string();

    // Upsert the (team, account) pair — re-connect rotates the token.
    sqlx::query(
        "INSERT INTO cloudflare_connection
         (id, team_id, account_id, account_name, auth_method, api_token,
          created_by_user_id)
         VALUES ($1,$2,$3,$4,'api_token',$5,$6)
         ON CONFLICT (team_id, account_id) DO UPDATE SET
           api_token = EXCLUDED.api_token,
           account_name = EXCLUDED.account_name,
           updated_at = now()",
    )
    .bind(runway_core::slugify::token_hex(16))
    .bind(&team_id)
    .bind(&account_id)
    .bind(&account_name)
    .bind(
        state
            .crypto
            .encrypt(&body.api_token)
            .map_err(ApiError::from)?,
    )
    .bind(user.user.id)
    .execute(&state.db)
    .await?;

    // Tunnel create + container start — same shape as devpush _ensure_tunnel.
    let conn = connection(&state, &team_id).await?.expect("just upserted");
    ensure_tunnel(&state, conn).await?;

    Ok(StatusCode::CREATED.into_response())
}

pub async fn disconnect(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> ApiResult<Response> {
    let (_, role) = team_role(&state, user.user.id, &team_id).await?;
    require_admin(&role)?;
    let Some(conn) = connection(&state, &team_id).await? else {
        return Err(ApiError::not_found("cloudflare connection"));
    };

    if let Some(d) = &state.docker {
        let _ = runway_core::tunnel::stop_cloudflared(d, &container_name(&team_id)).await;
    }
    if let Some(tunnel_id) = &conn.tunnel_id {
        if let Ok(token) = conn.api_token_dec(&state.crypto) {
            let cf = runway_core::cloudflare::CloudflareClient::new(token);
            let _ = cf.delete_tunnel(&conn.account_id, tunnel_id).await;
        }
    }
    // Clear DNS bookkeeping on this team's domains (parity with devpush).
    sqlx::query(
        "UPDATE domain SET cloudflare_zone_id = NULL, cloudflare_record_id = NULL
         WHERE project_id IN (SELECT id FROM project WHERE team_id = $1)
           AND cloudflare_record_id IS NOT NULL",
    )
    .bind(&team_id)
    .execute(&state.db)
    .await?;
    sqlx::query("DELETE FROM cloudflare_connection WHERE id = $1")
        .bind(&conn.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })).into_response())
}

pub async fn zones(
    user: AuthUser,
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> ApiResult<Response> {
    team_role(&state, user.user.id, &team_id).await?;
    let Some(conn) = connection(&state, &team_id).await? else {
        return Err(ApiError::not_found("cloudflare connection"));
    };
    let cf = runway_core::cloudflare::CloudflareClient::new(
        conn.api_token_dec(&state.crypto).map_err(ApiError::from)?,
    );
    let zones = cf.list_zones().await.map_err(ApiError::from)?;
    Ok(Json(json!({ "zones": zones })).into_response())
}

/// devpush `_ensure_tunnel`: create tunnel via CF API, persist encrypted
/// token, start `cloudflared-<team_id>` on Traefik's network.
pub async fn ensure_tunnel(
    state: &AppState,
    conn: runway_core::models::CloudflareConnection,
) -> ApiResult<()> {
    if conn.has_tunnel() {
        return Ok(());
    }
    let token = conn.api_token_dec(&state.crypto).map_err(ApiError::from)?;
    let cf = runway_core::cloudflare::CloudflareClient::new(token);
    let name = format!("runway-{}", &conn.team_id[..conn.team_id.len().min(8)]);
    let tunnel = cf
        .create_tunnel(&conn.account_id, &name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::internal("cloudflare tunnel create failed"))?;

    sqlx::query(
        "UPDATE cloudflare_connection SET tunnel_id = $1, tunnel_name = $2,
         tunnel_token = $3, updated_at = now() WHERE id = $4",
    )
    .bind(&tunnel.id)
    .bind(&tunnel.name)
    .bind(
        state
            .crypto
            .encrypt(&tunnel.token)
            .map_err(ApiError::from)?,
    )
    .bind(&conn.id)
    .execute(&state.db)
    .await?;

    if let Some(docker) = &state.docker {
        if let Some(net) = runway_core::tunnel::traefik_network(docker).await {
            match runway_core::tunnel::ensure_cloudflared(
                docker,
                &container_name(&conn.team_id),
                &tunnel.token,
                &net,
            )
            .await
            {
                Ok(cid) => {
                    sqlx::query(
                        "UPDATE cloudflare_connection SET tunnel_container_id = $1
                         WHERE id = $2",
                    )
                    .bind(cid)
                    .bind(&conn.id)
                    .execute(&state.db)
                    .await?;
                }
                Err(e) => tracing::warn!(error = %e, "team cloudflared start failed"),
            }
        } else {
            tracing::warn!("traefik container not found — team tunnel has no network");
        }
    }
    Ok(())
}

pub fn container_name(team_id: &str) -> String {
    format!("cloudflared-{team_id}")
}
