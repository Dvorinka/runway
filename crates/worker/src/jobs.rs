//! Postgres job queue — `SELECT ... FOR UPDATE SKIP LOCKED`.
//! Replaces taskiq+Redis. Job kinds map to functions in `deploy.rs`.

use std::time::Duration;

use chrono::Duration as ChronoDuration;
use serde_json::Value;
use sqlx::PgPool;

use crate::deploy;
use crate::Ctx;

const BATCH: i64 = 8;
const POLL_MS: u64 = 500;
const MAX_ATTEMPTS: i32 = 3;

pub async fn run(ctx: Ctx) {
    let mut ticker = tokio::time::interval(Duration::from_millis(POLL_MS));
    loop {
        ticker.tick().await;
        if let Err(e) = tick(&ctx).await {
            tracing::error!(error = %e, "job loop tick failed");
        }
    }
}

async fn tick(ctx: &Ctx) -> anyhow::Result<()> {
    // Claim a batch: lock rows, mark running.
    let jobs: Vec<(i64, String, Value, i32)> = {
        let mut tx = ctx.db.begin().await?;
        let rows = sqlx::query_as::<_, (i64, String, Value, i32)>(
            "SELECT id, kind, payload, attempts FROM job
             WHERE status = 'pending' AND run_at <= now()
             ORDER BY run_at
             LIMIT $1
             FOR UPDATE SKIP LOCKED",
        )
        .bind(BATCH)
        .fetch_all(&mut *tx)
        .await?;
        for (id, _, _, _) in &rows {
            sqlx::query("UPDATE job SET status = 'running', locked_at = now() WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        rows
    };

    for (id, kind, payload, attempts) in jobs {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            match dispatch(&ctx, &kind, &payload).await {
                Ok(()) => {
                    let _ = sqlx::query(
                        "UPDATE job SET status = 'done', updated_at = now() WHERE id = $1",
                    )
                    .bind(id)
                    .execute(&ctx.db)
                    .await;
                }
                Err(e) => {
                    tracing::warn!(job_id = id, kind, error = %e, "job failed");
                    let retry = attempts + 1 < MAX_ATTEMPTS;
                    let _ = if retry {
                        sqlx::query(
                            "UPDATE job SET status = 'pending', attempts = attempts + 1,
                             last_error = $2, run_at = now() + interval '5 seconds' * $3,
                             updated_at = now()
                             WHERE id = $1",
                        )
                        .bind(id)
                        .bind(e.to_string())
                        .bind((attempts + 1) as i64)
                        .execute(&ctx.db)
                        .await
                    } else {
                        sqlx::query(
                            "UPDATE job SET status = 'failed', attempts = attempts + 1,
                             last_error = $2, updated_at = now() WHERE id = $1",
                        )
                        .bind(id)
                        .bind(e.to_string())
                        .execute(&ctx.db)
                        .await
                    };
                }
            }
        });
    }
    Ok(())
}

async fn dispatch(ctx: &Ctx, kind: &str, payload: &Value) -> anyhow::Result<()> {
    match kind {
        "start_deployment" => deploy::start(ctx, str_payload(payload, "deployment_id")?).await,
        "finalize_deployment" => {
            deploy::finalize(ctx, str_payload(payload, "deployment_id")?).await
        }
        "fail_deployment" => {
            deploy::fail(
                ctx,
                str_payload(payload, "deployment_id")?,
                payload
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("deploy"),
                payload.get("reason").and_then(|v| v.as_str()),
            )
            .await
        }
        "delete_container" => {
            deploy::delete_container(ctx, str_payload(payload, "deployment_id")?).await
        }
        "cleanup_inactive_containers" => {
            deploy::cleanup_inactive(ctx, str_payload(payload, "project_id")?).await
        }
        "reconcile_edge_network" => {
            deploy::reconcile_edge_network(ctx, str_payload_opt(payload, "deployment_id")).await
        }
        "ensure_instance_tunnel" => crate::tunnel::ensure_instance(ctx).await,
        "teardown_instance_tunnel" => crate::tunnel::teardown_instance(ctx).await,
        "provision_storage" => {
            crate::storage::provision(ctx, str_payload(payload, "storage_id")?).await
        }
        "deprovision_storage" => {
            crate::storage::deprovision(ctx, str_payload(payload, "storage_id")?).await
        }
        "reset_storage" => crate::storage::reset(ctx, str_payload(payload, "storage_id")?).await,
        other => anyhow::bail!("unknown job kind: {other}"),
    }
}

fn str_payload<'a>(payload: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("job payload missing '{key}'"))
}

fn str_payload_opt(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(|v| v.as_str()).map(String::from)
}

pub async fn enqueue_deferred(
    db: &PgPool,
    kind: &str,
    payload: Value,
    defer_seconds: i64,
) -> anyhow::Result<i64> {
    let run_at = chrono::Utc::now() + ChronoDuration::seconds(defer_seconds);
    let (id,): (i64,) =
        sqlx::query_as("INSERT INTO job (kind, payload, run_at) VALUES ($1,$2,$3) RETURNING id")
            .bind(kind)
            .bind(payload)
            .bind(run_at)
            .fetch_one(db)
            .await?;
    Ok(id)
}
