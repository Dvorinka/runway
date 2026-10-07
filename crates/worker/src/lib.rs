//! runway-worker: deployment pipeline and background loops.
//!
//! Single-binary model: the CLI's `runway serve` spawns these loops as
//! tokio tasks alongside the API. Postgres is the queue — deployments are
//! claimed with `SELECT ... FOR UPDATE SKIP LOCKED`, no Redis required.
//!
//! Loops:
//! - `deploy`:  claim queued deployments -> build/run containers -> probe -> finalize.
//! - `monitor`: re-check running containers; reconcile observed state.
//! - `cron`:    due jobs trigger redeploys / HTTP calls.

use std::time::Duration;

use sqlx::PgPool;

/// Run all worker loops until shutdown. Stubs for now — the deploy
/// pipeline lands in Phase 1.
pub async fn run(db: PgPool, settings: runway_core::Settings) -> anyhow::Result<()> {
    let interval = Duration::from_secs(settings.monitor_interval_seconds.max(1));
    tracing::info!(?interval, "worker loops started");
    let _ = db; // jobs land with the deploy pipeline
    loop {
        tokio::time::sleep(interval).await;
        // TODO(phase-1): claim queued deployments, probe running containers.
    }
}
