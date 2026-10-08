use bollard::Docker;
use runway_core::crypto::Crypto;
use runway_core::events::EventBus;
use runway_core::github::GithubService;
use runway_core::logs::LogStore;
use runway_core::Settings;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub settings: Settings,
    pub bus: EventBus,
    pub crypto: Crypto,
    /// Always present — `configured()` reports whether app credentials
    /// exist (env or DB-registered); registration hot-patches in place.
    pub github: GithubService,
    pub logs: LogStore,
    /// Optional — Cloudflare tunnel handlers degrade when absent.
    pub docker: Option<Docker>,
}
