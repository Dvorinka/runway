//! Cascading deletes — ports of devpush `tasks/project.py`,
//! `tasks/team.py`, `tasks/user.py`. The API marks the row deleted and
//! enqueues; the job tears down containers, aliases, edge config, child
//! rows, then the row itself.

use crate::{deploy, storage, Ctx};

/// `delete_project` job — expects the project already marked `deleted`.
/// Batches deployments (container → alias → row), then project children
/// and the project row. Port of tasks/project.py:delete_project.
pub async fn delete_project(ctx: &Ctx, project_id: &str) -> anyhow::Result<()> {
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM project WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&ctx.db)
        .await?;
    match status.as_deref() {
        Some("deleted") => {}
        Some(_) => anyhow::bail!("project {project_id} is not marked as deleted"),
        None => return Ok(()),
    }

    // Deployment batches: container teardown reuses delete_container
    // (edge network + remote-node handling included).
    loop {
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM deployment WHERE project_id = $1 LIMIT 100")
                .bind(project_id)
                .fetch_all(&ctx.db)
                .await?;
        if ids.is_empty() {
            break;
        }
        for dep_id in &ids {
            if let Err(e) = deploy::delete_container(ctx, dep_id).await {
                tracing::warn!(deployment_id = dep_id, error = %e, "delete_project: container teardown failed");
            }
        }
        sqlx::query("DELETE FROM alias WHERE deployment_id = ANY($1)")
            .bind(&ids)
            .execute(&ctx.db)
            .await?;
        sqlx::query("DELETE FROM deployment WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&ctx.db)
            .await?;
    }

    runway_core::traefik::remove_project_config(project_id, &ctx.settings).await?;
    for table in [
        "domain",
        "deploy_token",
        "project_webhook",
        "storage_project",
        "notification",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE project_id = $1"))
            .bind(project_id)
            .execute(&ctx.db)
            .await?;
    }
    sqlx::query("DELETE FROM project WHERE id = $1")
        .bind(project_id)
        .execute(&ctx.db)
        .await?;
    tracing::info!(project_id, "project deleted");
    Ok(())
}

/// `delete_team` job — expects the team marked `deleted`. Deletes every
/// project (mark + delete_project), deprovisions team storage, removes
/// memberships/invites, then the team row. Port of tasks/team.py.
pub async fn delete_team(ctx: &Ctx, team_id: &str) -> anyhow::Result<()> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM team WHERE id = $1")
        .bind(team_id)
        .fetch_optional(&ctx.db)
        .await?;
    if exists.is_none() {
        return Ok(());
    }

    let project_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM project WHERE team_id = $1 AND status != 'deleted'")
            .bind(team_id)
            .fetch_all(&ctx.db)
            .await?;
    for pid in project_ids {
        sqlx::query("UPDATE project SET status = 'deleted', updated_at = now() WHERE id = $1")
            .bind(&pid)
            .execute(&ctx.db)
            .await?;
        delete_project(ctx, &pid).await?;
    }

    let storage_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM storage WHERE team_id = $1")
        .bind(team_id)
        .fetch_all(&ctx.db)
        .await?;
    for sid in storage_ids {
        if let Err(e) = storage::deprovision(ctx, &sid).await {
            tracing::warn!(storage_id = sid, error = %e, "delete_team: storage deprovision failed");
        }
    }

    // Clear default_team_id on users pointing at this team.
    sqlx::query("UPDATE \"user\" SET default_team_id = NULL WHERE default_team_id = $1")
        .bind(team_id)
        .execute(&ctx.db)
        .await?;
    for sql in [
        "DELETE FROM team_member WHERE team_id = $1",
        "DELETE FROM team_invite WHERE team_id = $1",
        "DELETE FROM team_webhook WHERE team_id = $1",
        "DELETE FROM cloudflare_connection WHERE team_id = $1",
        "DELETE FROM team WHERE id = $1",
    ] {
        sqlx::query(sql).bind(team_id).execute(&ctx.db).await?;
    }
    tracing::info!(team_id, "team deleted");
    Ok(())
}

/// `delete_user` job — expects the user marked `deleted`. Teams where the
/// user is the sole owner are deleted with it; other memberships are
/// just dropped. Port of tasks/user.py.
pub async fn delete_user(ctx: &Ctx, user_id: i64) -> anyhow::Result<()> {
    let email: Option<String> = sqlx::query_scalar("SELECT email FROM \"user\" WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&ctx.db)
        .await?;
    let Some(email) = email else { return Ok(()) };

    // Teams where this user is the only owner would be orphaned — delete.
    let owned_teams: Vec<String> = sqlx::query_scalar(
        "SELECT tm.team_id FROM team_member tm WHERE tm.user_id = $1 AND tm.role = 'owner'
         AND (SELECT count(*) FROM team_member o
              WHERE o.team_id = tm.team_id AND o.role = 'owner') = 1",
    )
    .bind(user_id)
    .fetch_all(&ctx.db)
    .await?;
    for tid in &owned_teams {
        sqlx::query("UPDATE team SET status = 'deleted', updated_at = now() WHERE id = $1")
            .bind(tid)
            .execute(&ctx.db)
            .await?;
        delete_team(ctx, tid).await?;
    }

    sqlx::query("DELETE FROM team_member WHERE user_id = $1")
        .bind(user_id)
        .execute(&ctx.db)
        .await?;
    sqlx::query("DELETE FROM team_invite WHERE inviter_id = $1 OR email = $2")
        .bind(user_id)
        .bind(&email)
        .execute(&ctx.db)
        .await?;
    sqlx::query("DELETE FROM user_identity WHERE user_id = $1")
        .bind(user_id)
        .execute(&ctx.db)
        .await?;
    sqlx::query("DELETE FROM api_key WHERE user_id = $1")
        .bind(user_id)
        .execute(&ctx.db)
        .await?;
    sqlx::query("DELETE FROM \"user\" WHERE id = $1")
        .bind(user_id)
        .execute(&ctx.db)
        .await?;
    tracing::info!(user_id, "user deleted");
    Ok(())
}
