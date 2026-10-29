//! Retention sweep: prunes rows that would otherwise grow forever.
//!
//! devpush has no analog — analytics and magic-link tokens are
//! Runway additions — so bounds are chosen for a self-hosted box:
//! analytics 90d, expired single-use tokens, read notifications 30d.

use crate::Ctx;

/// How often the sweep runs. First tick fires immediately on boot.
const SWEEP_SECONDS: u64 = 24 * 3600;

/// Spawn the retention loop.
pub fn spawn(ctx: Ctx) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(SWEEP_SECONDS));
        loop {
            ticker.tick().await;
            if let Err(e) = sweep(&ctx).await {
                tracing::warn!(error = %e, "retention sweep failed");
            }
        }
    });
}

async fn sweep(ctx: &Ctx) -> anyhow::Result<()> {
    let a = sqlx::query("DELETE FROM analytics_event WHERE ts < now() - interval '90 days'")
        .execute(&ctx.db)
        .await?
        .rows_affected();
    let e = sqlx::query(
        "DELETE FROM email_token
         WHERE expires_at < now() OR used_at < now() - interval '7 days'",
    )
    .execute(&ctx.db)
    .await?
    .rows_affected();
    let n = sqlx::query(
        "DELETE FROM node_token
         WHERE expires_at < now() - interval '7 days'
            OR revoked_at < now() - interval '7 days'",
    )
    .execute(&ctx.db)
    .await?
    .rows_affected();
    let r = sqlx::query(
        "DELETE FROM notification
         WHERE read AND created_at < now() - interval '30 days'",
    )
    .execute(&ctx.db)
    .await?
    .rows_affected();
    if a + e + n + r > 0 {
        tracing::info!(
            analytics = a,
            email_tokens = e,
            node_tokens = n,
            notifications = r,
            "retention sweep pruned rows"
        );
    }
    Ok(())
}
