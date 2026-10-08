//! Cron tick loop: fires scheduled deployments.
//!
//! Port of devpush `workers/tasks/cron.py`: every minute, find enabled
//! jobs whose `next_run_at` lapsed, resolve the branch head, and create
//! a deployment. `next_run_at` advances by the parsed interval even on
//! failure so a broken job doesn't wedge the loop.

use crate::Ctx;

/// Interval between cron sweeps.
const TICK_SECONDS: u64 = 60;

type DueJob = (
    String,         // id
    String,         // project_id
    String,         // schedule
    String,         // branch
    Option<String>, // environment_id
);

/// Spawn the cron loop.
pub fn spawn(ctx: Ctx) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(TICK_SECONDS));
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&ctx).await {
                tracing::warn!(error = %e, "cron tick failed");
            }
        }
    });
}

async fn tick(ctx: &Ctx) -> anyhow::Result<()> {
    let due: Vec<DueJob> = sqlx::query_as(
        "SELECT id, project_id, schedule, branch, environment_id
         FROM cron_job
         WHERE enabled AND next_run_at <= now()
         ORDER BY next_run_at",
    )
    .fetch_all(&ctx.db)
    .await?;

    for (job_id, project_id, schedule, branch, _env_id) in due {
        let interval = runway_core::cron::parse_schedule(&schedule).max(1);
        // Advance the job first — whether or not the deploy succeeds it
        // must not fire again on the next tick.
        sqlx::query(
            "UPDATE cron_job SET last_run_at = now(),
                next_run_at = now() + make_interval(mins => $2)
             WHERE id = $1",
        )
        .bind(&job_id)
        .bind(interval as i32)
        .execute(&ctx.db)
        .await?;

        let project: Option<runway_core::models::Project> =
            sqlx::query_as("SELECT * FROM project WHERE id = $1 AND status = 'active'")
                .bind(&project_id)
                .fetch_optional(&ctx.db)
                .await?;
        let Some(project) = project else {
            tracing::warn!(job = %job_id, "cron: project gone, disabling");
            sqlx::query("UPDATE cron_job SET enabled = false WHERE id = $1")
                .bind(&job_id)
                .execute(&ctx.db)
                .await?;
            continue;
        };

        let result = runway_core::deploy::resolve_commit(
            &ctx.db,
            &ctx.crypto,
            ctx.github.if_configured(),
            &project,
            &branch,
        )
        .await;
        let commit = match result {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(job = %job_id, project = %project_id, error = %e,
                    "cron: commit resolution failed");
                continue;
            }
        };
        match runway_core::deploy::create(
            &ctx.db,
            &ctx.bus,
            &ctx.crypto,
            &project,
            &branch,
            &commit,
            "cron",
            None,
            None,
        )
        .await
        {
            Ok(dep) => tracing::info!(job = %job_id, deployment = %dep.id, "cron deployed"),
            Err(e) => tracing::warn!(job = %job_id, error = %e, "cron deploy create failed"),
        }
    }
    Ok(())
}
