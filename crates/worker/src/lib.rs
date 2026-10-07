//! runway-worker: deployment pipeline and background loops.
//!
//! Single-binary model: `runway serve` spawns these loops as tokio tasks
//! alongside the API. Postgres is the queue — jobs are claimed with
//! `SELECT ... FOR UPDATE SKIP LOCKED`, no Redis required.
//!
//! Loops:
//! - `jobs`:    claim queued jobs -> deploy pipeline handlers.
//! - `monitor`: probe containers in `deploy`, sweep observed state.

pub mod deploy;
pub mod jobs;
pub mod monitor;
pub mod tunnel;

use bollard::Docker;
use sqlx::PgPool;

use runway_core::crypto::Crypto;
use runway_core::events::EventBus;
use runway_core::github::GithubService;
use runway_core::logs::LogStore;
use runway_core::Settings;

/// Shared worker context passed to every loop/job.
#[derive(Clone)]
pub struct Ctx {
    pub db: PgPool,
    pub settings: Settings,
    pub docker: Docker,
    pub crypto: Crypto,
    pub bus: EventBus,
    pub logs: LogStore,
    pub github: Option<GithubService>,
}

/// Run all worker loops until shutdown.
pub async fn run(
    db: PgPool,
    settings: Settings,
    bus: EventBus,
    crypto: Crypto,
) -> anyhow::Result<()> {
    let docker = runway_core::docker::connect(&settings)?;
    let ctx = Ctx {
        db: db.clone(),
        settings: settings.clone(),
        docker,
        crypto,
        bus: bus.clone(),
        logs: LogStore::new(&settings.data_dir, bus),
        github: GithubService::from_settings(&settings),
    };

    // Attach Traefik to edge networks from before a restart.
    let startup_ctx = ctx.clone();
    tokio::spawn(async move {
        if let Err(e) = deploy::reconcile_edge_network(&startup_ctx, None).await {
            tracing::warn!(error = %e, "startup edge reconcile failed");
        }
    });

    // CGNAT path: bring up the instance-level Cloudflare Tunnel when
    // CF_API_TOKEN + CF_ACCOUNT_ID are configured.
    if settings.cf_configured() {
        if let Err(e) =
            runway_core::deploy::enqueue(&db, "ensure_instance_tunnel", serde_json::json!({}), 0)
                .await
        {
            tracing::warn!(error = %e, "failed to enqueue instance tunnel setup");
        }
        // Restart covers crashes; this loop covers a removed container
        // or a changed Traefik network (re-enqueues a full reconcile).
        let tunnel_ctx = ctx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                ticker.tick().await;
                let status = runway_core::tunnel::cloudflared_status(
                    &tunnel_ctx.docker,
                    "cloudflared-instance",
                )
                .await;
                if status != "running" {
                    tracing::warn!(status, "instance tunnel container down — re-enqueueing");
                    let _ = runway_core::deploy::enqueue(
                        &tunnel_ctx.db,
                        "ensure_instance_tunnel",
                        serde_json::json!({}),
                        0,
                    )
                    .await;
                }
            }
        });
    }

    // Per-team tunnels: restart any `cloudflared-<team>` container that
    // vanished (restart policy covers crashes; this covers removal).
    {
        let team_ctx = ctx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                ticker.tick().await;
                let conns: Vec<runway_core::models::CloudflareConnection> = match sqlx::query_as(
                    "SELECT * FROM cloudflare_connection WHERE tunnel_id IS NOT NULL",
                )
                .fetch_all(&team_ctx.db)
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(error = %e, "team tunnel health query failed");
                        continue;
                    }
                };
                for conn in conns {
                    let name = format!("cloudflared-{}", conn.team_id);
                    let status =
                        runway_core::tunnel::cloudflared_status(&team_ctx.docker, &name).await;
                    if status == "running" {
                        continue;
                    }
                    let Ok(Some(token)) = conn.tunnel_token_dec(&team_ctx.crypto) else {
                        tracing::warn!(team = %conn.team_id, "team tunnel: no token, skipping");
                        continue;
                    };
                    let Some(net) = runway_core::tunnel::traefik_network(&team_ctx.docker).await
                    else {
                        tracing::warn!("team tunnel: traefik network not found");
                        continue;
                    };
                    tracing::warn!(team = %conn.team_id, status, "restarting team tunnel container");
                    match runway_core::tunnel::ensure_cloudflared(
                        &team_ctx.docker,
                        &name,
                        &token,
                        &net,
                    )
                    .await
                    {
                        Ok(cid) => {
                            let _ = sqlx::query(
                                "UPDATE cloudflare_connection SET tunnel_container_id = $1
                                 WHERE id = $2",
                            )
                            .bind(&cid)
                            .bind(&conn.id)
                            .execute(&team_ctx.db)
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, team = %conn.team_id, "team tunnel restart failed")
                        }
                    }
                }
            }
        });
    }

    let jobs_ctx = ctx.clone();
    tokio::spawn(async move { jobs::run(jobs_ctx).await });

    tokio::spawn(async move { monitor::run(ctx).await });

    tracing::info!("worker loops started (jobs, monitor)");
    Ok(())
}
