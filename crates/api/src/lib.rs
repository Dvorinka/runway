//! runway-api: axum HTTP layer.
//!
//! Serves the REST API, the React SPA (static files), SSE streams for
//! deployment status/logs, git provider webhooks, and the MCP endpoint.

pub mod auth;
pub mod error;
pub mod routes;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::json;

pub use state::AppState;

pub fn router(state: AppState) -> Router {
    let web_dir = std::path::PathBuf::from(&state.settings.web_dir);
    Router::new()
        .route("/health", get(health))
        // Auth — email + password
        .route("/api/auth/login", post(routes::auth::login))
        .route("/api/auth/register", post(routes::auth::register))
        .route("/api/auth/logout", post(routes::auth::logout))
        .route(
            "/api/auth/me",
            get(routes::auth::me)
                .patch(routes::auth::update_me)
                .delete(routes::auth::delete_me),
        )
        .route("/api/auth/password", post(routes::auth::change_password))
        .route(
            "/api/auth/avatar",
            put(routes::avatars::put_user).delete(routes::avatars::delete_user),
        )
        .route("/api/avatars/{kind}/{id}", get(routes::avatars::get))
        // OIDC / SSO
        .route("/api/auth/oidc", get(routes::oidc::authorize))
        .route("/api/auth/oidc/callback", get(routes::oidc::callback))
        .route("/api/auth/oidc/info", get(routes::oidc::info))
        // Admin — sign-up allowlist
        .route(
            "/api/v1/admin/allowlist",
            get(routes::admin::list_allowlist).post(routes::admin::add_allowlist_rule),
        )
        .route(
            "/api/v1/admin/allowlist/{id}",
            delete(routes::admin::delete_allowlist_rule),
        )
        // Admin — remote Docker nodes
        .route(
            "/api/v1/admin/nodes",
            get(routes::admin::list_nodes).post(routes::admin::create_node),
        )
        .route(
            "/api/v1/admin/nodes/{id}",
            delete(routes::admin::delete_node),
        )
        .route(
            "/api/v1/admin/nodes/{id}/health",
            post(routes::admin::check_node),
        )
        // GitHub integration — webhook + app-manifest registration
        .route("/api/github/webhook", post(routes::github::webhook))
        .route("/api/v1/github/app/status", get(routes::github::app_status))
        .route(
            "/api/v1/github/app/register",
            get(routes::github::app_register),
        )
        .route(
            "/api/v1/github/app/callback",
            get(routes::github::app_callback),
        )
        // Other git providers — connection CRUD + inbound webhooks
        .route(
            "/api/gitea/webhook",
            post(routes::git_providers::gitea_webhook),
        )
        .route(
            "/api/gitlab/webhook",
            post(routes::git_providers::gitlab_webhook),
        )
        .route(
            "/api/bitbucket/webhook",
            post(routes::git_providers::bitbucket_webhook),
        )
        .route(
            "/api/v1/git/{provider}/connect",
            post(routes::git_providers::connect),
        )
        .route(
            "/api/v1/git/{provider}/connections",
            get(routes::git_providers::list_connections),
        )
        .route(
            "/api/v1/git/{provider}/connections/{conn_id}",
            delete(routes::git_providers::delete_connection),
        )
        .route(
            "/api/v1/git/{provider}/connections/{conn_id}/repos",
            get(routes::git_providers::list_repos),
        )
        .route(
            "/api/v1/git/{provider}/connections/{conn_id}/branches/{*full}",
            get(routes::git_providers::list_branches),
        )
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
            get(routes::projects::get)
                .patch(routes::projects::patch)
                .delete(routes::projects::delete),
        )
        .route(
            "/api/v1/projects/{id}/avatar",
            put(routes::avatars::put_project).delete(routes::avatars::delete_project),
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
        // Cron jobs + redirect rules
        .route(
            "/api/v1/projects/{id}/cron",
            get(routes::projects::list_cron).post(routes::projects::create_cron),
        )
        .route(
            "/api/v1/projects/{id}/cron/{job_id}",
            axum::routing::patch(routes::projects::patch_cron)
                .delete(routes::projects::delete_cron),
        )
        .route(
            "/api/v1/projects/{id}/redirects",
            get(routes::projects::list_redirects).post(routes::projects::create_redirect),
        )
        .route(
            "/api/v1/projects/{id}/redirects/{rid}",
            axum::routing::patch(routes::projects::patch_redirect)
                .delete(routes::projects::delete_redirect),
        )
        // Speed insights — beacon (public, host-routed) + aggregates.
        .route("/api/v1/status/{slug}", get(routes::status::status_page))
        .route("/_runway-rum", post(routes::rum::beacon))
        .route("/api/v1/rum", post(routes::rum::beacon))
        .route(
            "/api/v1/projects/{id}/logs",
            get(routes::deployments::project_logs),
        )
        .route("/api/v1/projects/{id}/speed", get(routes::rum::speed))
        .route(
            "/api/v1/projects/{id}/analytics",
            get(routes::rum::analytics),
        )
        // Project export / import
        .route(
            "/api/v1/projects/{id}/export",
            get(routes::projects::export_project),
        )
        .route(
            "/api/v1/projects/{id}/import",
            post(routes::projects::import_project),
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
            "/api/v1/teams/{id}/avatar",
            put(routes::avatars::put_team).delete(routes::avatars::delete_team),
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
        // Team storage (Phase 5)
        .route(
            "/api/v1/teams/{id}/storage",
            get(routes::storage::list).post(routes::storage::create),
        )
        .route(
            "/api/v1/teams/{id}/storage/{storage_id}",
            get(routes::storage::get).delete(routes::storage::delete),
        )
        .route(
            "/api/v1/teams/{id}/storage/{storage_id}/reset",
            axum::routing::post(routes::storage::reset),
        )
        .route(
            "/api/v1/teams/{id}/storage/{storage_id}/link",
            axum::routing::post(routes::storage::link),
        )
        .route(
            "/api/v1/teams/{id}/storage/{storage_id}/link/{project_id}",
            delete(routes::storage::unlink),
        )
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
        .route("/api/v1/deployments", get(routes::deployments::index))
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
        .route(
            "/api/v1/deployments/{id}/stats",
            get(routes::deployments::stats),
        )
        .route(
            "/api/v1/deployments/{id}/metrics",
            get(routes::deployments::metrics),
        )
        .route(
            "/api/v1/deployments/{id}/reconcile",
            post(routes::deployments::reconcile),
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
