//! runway-api: axum HTTP layer.
//!
//! Serves the REST API, the React SPA (static files), SSE streams for
//! deployment status/logs, git provider webhooks, and the MCP endpoint.

pub mod auth;
pub mod error;
pub mod routes;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;

pub use state::AppState;

pub fn router(state: AppState) -> Router {
    let web_dir = std::path::PathBuf::from(&state.settings.web_dir);
    Router::new()
        .route("/health", get(health))
        // Auth
        .route("/api/auth/github", get(routes::auth::github_login))
        .route(
            "/api/auth/github/callback",
            get(routes::auth::github_callback),
        )
        .route("/api/auth/logout", post(routes::auth::logout))
        .route("/api/auth/me", get(routes::auth::me))
        .route("/api/auth/magic-link", post(routes::auth::magic_link))
        .route(
            "/api/auth/magic-link/verify",
            get(routes::auth::magic_link_verify),
        )
        // GitHub integration
        .route("/api/github/webhook", post(routes::github::webhook))
        .route(
            "/api/v1/github/installations",
            get(routes::github::installations),
        )
        .route(
            "/api/v1/github/installations/{id}/repos",
            get(routes::github::installation_repos),
        )
        // Projects
        .route(
            "/api/v1/projects",
            get(routes::projects::list).post(routes::projects::create),
        )
        .route(
            "/api/v1/projects/{id}",
            get(routes::projects::get).patch(routes::projects::patch),
        )
        .route(
            "/api/v1/projects/{id}/env",
            get(routes::projects::get_env)
                .put(routes::projects::put_env)
                .patch(routes::projects::patch_env),
        )
        .route(
            "/api/v1/projects/{id}/deploy-tokens",
            post(routes::projects::create_deploy_token),
        )
        .route(
            "/api/v1/projects/{id}/deploy-tokens/{token_id}",
            delete(routes::projects::delete_deploy_token),
        )
        .route(
            "/api/v1/projects/{id}/domains",
            get(routes::projects::list_domains).post(routes::projects::add_domain),
        )
        .route(
            "/api/v1/projects/{id}/domains/{domain_id}/verify",
            post(routes::projects::verify_domain),
        )
        .route(
            "/api/v1/projects/{id}/domains/{domain_id}/assign-cloudflare",
            post(routes::projects::assign_cloudflare_domain),
        )
        .route(
            "/api/v1/projects/{id}/domains/{domain_id}",
            delete(routes::projects::delete_domain),
        )
        // Deployments
        .route(
            "/api/v1/projects/{id}/deployments",
            get(routes::deployments::list).post(routes::deployments::create),
        )
        .route(
            "/api/v1/projects/{id}/environments/{env_id}/rollback",
            post(routes::deployments::rollback),
        )
        .route(
            "/api/v1/projects/{id}/webhooks",
            get(routes::projects::list_webhooks).post(routes::projects::create_webhook),
        )
        .route(
            "/api/v1/projects/{id}/webhooks/{webhook_id}",
            delete(routes::projects::delete_webhook),
        )
        // Teams + team webhooks
        .route(
            "/api/v1/teams",
            get(routes::teams::list).post(routes::teams::create),
        )
        .route(
            "/api/v1/teams/{id}",
            get(routes::teams::get)
                .patch(routes::teams::update)
                .delete(routes::teams::delete),
        )
        .route(
            "/api/v1/teams/{id}/members/{user_id}",
            axum::routing::patch(routes::teams::update_member).delete(routes::teams::remove_member),
        )
        .route(
            "/api/v1/teams/{id}/invites",
            get(routes::teams::list_invites).post(routes::teams::create_invite),
        )
        .route(
            "/api/v1/teams/{id}/invites/{invite_id}",
            delete(routes::teams::revoke_invite),
        )
        .route(
            "/api/v1/invites/{id}/accept",
            axum::routing::post(routes::teams::accept_invite),
        )
        .route(
            "/api/v1/teams/{id}/audit",
            get(routes::notifications::team_audit),
        )
        .route("/api/v1/notifications", get(routes::notifications::list))
        .route(
            "/api/v1/notifications/mark-read",
            axum::routing::post(routes::notifications::mark_read),
        )
        .route("/api/deploy", get(routes::notifications::deploy_button))
        .route(
            "/api/v1/teams/{id}/webhooks",
            get(routes::teams::list_webhooks).post(routes::teams::create_webhook),
        )
        .route(
            "/api/v1/teams/{id}/webhooks/{webhook_id}",
            delete(routes::teams::delete_webhook),
        )
        // Per-team Cloudflare connection + tunnel (Phase 3 remainder).
        .route(
            "/api/v1/teams/{id}/cloudflare",
            get(routes::cloudflare::status).delete(routes::cloudflare::disconnect),
        )
        .route(
            "/api/v1/teams/{id}/cloudflare/connect",
            axum::routing::post(routes::cloudflare::connect),
        )
        .route(
            "/api/v1/teams/{id}/cloudflare/zones",
            get(routes::cloudflare::zones),
        )
        // MCP (JSON-RPC 2.0) + OpenAPI
        .route("/api/mcp", post(routes::mcp::rpc))
        .route("/api/v1/openapi.json", get(routes::api::openapi))
        .route(
            "/api/v1/projects/{id}/events",
            get(routes::deployments::events),
        )
        .route(
            "/api/v1/projects/{id}/deployments/upload",
            post(routes::deployments::upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/v1/deployments/{id}", get(routes::deployments::get))
        .route(
            "/api/v1/deployments/{id}/cancel",
            post(routes::deployments::cancel),
        )
        .route(
            "/api/v1/deployments/{id}/skip",
            post(routes::deployments::skip),
        )
        .route(
            "/api/v1/deployments/{id}/redeploy",
            post(routes::deployments::redeploy),
        )
        .route(
            "/api/v1/deployments/{id}/logs",
            get(routes::deployments::logs),
        )
        .route(
            "/api/v1/deployments/{id}/logs/stream",
            get(routes::deployments::logs_stream),
        )
        // API keys + deploy-token deploy
        .route(
            "/api/v1/keys",
            get(routes::api::list_keys).post(routes::api::create_key),
        )
        .route("/api/v1/keys/{id}", delete(routes::api::revoke_key))
        .route("/api/deploy", post(routes::api::api_deploy))
        .with_state(state)
        // React SPA — static assets + client-side route fallback.
        .fallback_service(tower_http::services::ServeDir::new(&web_dir).fallback(
            tower_http::services::ServeFile::new(web_dir.join("index.html")),
        ))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "runway" }))
}
