//! runway-api: axum HTTP layer.
//!
//! Serves the REST API, the React SPA (static files), SSE streams for
//! deployment status/logs, git provider webhooks, and the MCP endpoint.

use axum::{routing::get, Json, Router};
use serde_json::json;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub settings: runway_core::Settings,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        // Phase 1+: /api/v1/*, /api/github/webhook, /api/deploy, /api/mcp, SPA fallback
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "runway" }))
}
