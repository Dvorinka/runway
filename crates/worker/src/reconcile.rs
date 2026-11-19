//! Periodic observed-state reconciler — port of devpush
//! `services/reconcile.py::reconcile_deployments` (the non-full-scan
//! branch). The monitor sweeps only `container_status='running' AND
//! conclusion='succeeded'` every couple seconds; this slower loop keeps
//! `observed_*` fresh for the rest — stopped containers, failed rows,
//! anything the fast sweep doesn't cover — so the dashboard shows true
//! state and `observed_missing_count` distinguishes blip from gone.

use std::collections::HashMap;

use bollard::container::ListContainersOptions;
use chrono::Utc;
use serde_json::json;

use runway_core::docker as dkr;
use runway_core::models::Deployment;

use crate::Ctx;

/// Port of devpush `reconcile_deployments_tick`.
pub fn spawn(ctx: Ctx) {
    tokio::spawn(async move {
        let secs = ctx.settings.reconcile_interval_seconds.max(5);
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs));
        // Traefik container recreation drops its dynamic edge-network
        // attachments; a changed id re-runs the attach sweep.
        let mut traefik_id = dkr::service_container_id(&ctx.docker, "traefik").await;
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&ctx).await {
                tracing::warn!(error = %e, "reconcile tick failed");
            }
            let now = dkr::service_container_id(&ctx.docker, "traefik").await;
            if now.is_some() && now != traefik_id {
                if traefik_id.is_some() {
                    tracing::info!("traefik recreated — reattaching edge networks");
                    if let Err(e) = crate::deploy::reconcile_edge_network(&ctx, None).await {
                        tracing::warn!(error = %e, "edge reattach failed");
                    }
                }
                traefik_id = now;
            }
        }
    });
}

async fn tick(ctx: &Ctx) -> anyhow::Result<()> {
    let deployments: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment
         WHERE container_status IN ('running','stopped')
            OR observed_status = 'running'",
    )
    .fetch_all(&ctx.db)
    .await?;
    if deployments.is_empty() {
        return Ok(());
    }

    // label → container map lets us find a container even when the
    // stored container_id is stale (devpush falls back the same way).
    let containers = ctx
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            filters: HashMap::from([(
                "label".to_string(),
                vec!["runway.deployment_id".to_string()],
            )]),
            ..Default::default()
        }))
        .await
        .unwrap_or_default();
    let mut by_deployment: HashMap<String, String> = HashMap::new();
    let mut by_id: HashMap<String, String> = HashMap::new();
    for c in containers {
        let Some(id) = c.id.clone() else { continue };
        by_id.insert(id.clone(), id.clone());
        if let Some(dep_id) = c
            .labels
            .as_ref()
            .and_then(|l| l.get("runway.deployment_id"))
        {
            by_deployment.insert(dep_id.clone(), id);
        }
    }

    let mut node_clients: HashMap<String, bollard::Docker> = HashMap::new();

    for dep in deployments {
        let docker = match dep.remote_node_id.as_deref() {
            Some(nid) => {
                if !node_clients.contains_key(nid) {
                    if let Some(c) = dkr::node_client(&ctx.db, nid, &ctx.crypto).await {
                        node_clients.insert(nid.to_string(), c);
                    }
                }
                match node_clients.get(nid) {
                    Some(c) => c,
                    // Node gone/unreachable — keep the previous
                    // observation rather than guessing.
                    None => continue,
                }
            }
            None => &ctx.docker,
        };

        // Stored id wins when it still exists on the daemon; otherwise
        // fall back to the label map (devpush identical).
        let container_id = dep
            .container_id
            .as_ref()
            .filter(|cid| dep.remote_node_id.is_some() || by_id.contains_key(*cid))
            .cloned()
            .or_else(|| by_deployment.get(&dep.id).cloned());

        let observed = match container_id {
            Some(cid) => dkr::inspect_observed(docker, &cid).await,
            None => Some(("not_found".to_string(), None)),
        };
        // Daemon error → keep the previous observation (devpush skips).
        let Some((observed, exit_code)) = observed else {
            continue;
        };
        let missing_count = if observed == "not_found" {
            dep.observed_missing_count + 1
        } else {
            0
        };
        let changed = dep.observed_status.as_deref() != Some(observed.as_str())
            || dep.observed_exit_code != exit_code.map(|c| c as i32)
            || dep.observed_missing_count != missing_count;

        sqlx::query(
            "UPDATE deployment SET observed_status = $1, observed_exit_code = $2,
             observed_at = $3, observed_last_seen_at = CASE WHEN $1 = 'not_found'
             THEN observed_last_seen_at ELSE $3 END,
             observed_missing_count = $4 WHERE id = $5",
        )
        .bind(&observed)
        .bind(exit_code)
        .bind(Utc::now())
        .bind(missing_count)
        .bind(&dep.id)
        .execute(&ctx.db)
        .await?;

        if changed {
            ctx.bus.publish(
                &format!("project:{}:updates", dep.project_id),
                &json!({
                    "event_type": "deployment_observed_update",
                    "project_id": dep.project_id,
                    "deployment_id": dep.id,
                    "observed_status": observed,
                    "observed_exit_code": exit_code,
                    "observed_missing_count": missing_count,
                    "timestamp": Utc::now().to_rfc3339(),
                }),
            );
        }
    }

    sweep_nodes(ctx).await;
    sweep_expired_previews(ctx).await;
    Ok(())
}

/// `preview_ttl_hours` per-project config: tear down PR-preview
/// deployments older than the TTL. Mirrors the api-crate PR-close
/// teardown (container → aliases → traefik rewrite). 0/absent = off.
async fn sweep_expired_previews(ctx: &Ctx) {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT d.id, d.project_id FROM deployment d
         JOIN project p ON p.id = d.project_id
         WHERE d.config ? 'pr_number'
           AND p.config->>'preview_ttl_hours' ~ '^[0-9]+$'
           AND (p.config->>'preview_ttl_hours')::int > 0
           AND d.created_at < now() - make_interval(hours => (p.config->>'preview_ttl_hours')::int)
           AND d.container_status != 'removed'",
    )
    .fetch_all(&ctx.db)
    .await
    .unwrap_or_default();
    let mut touched: Vec<String> = Vec::new();
    for (dep_id, project_id) in rows {
        if let Err(e) = crate::deploy::delete_container(ctx, &dep_id).await {
            tracing::warn!(deployment_id = dep_id, error = %e, "preview TTL teardown failed");
            continue;
        }
        let _ = sqlx::query("DELETE FROM alias WHERE deployment_id = $1")
            .bind(&dep_id)
            .execute(&ctx.db)
            .await;
        touched.push(project_id);
    }
    touched.sort_unstable();
    touched.dedup();
    for project_id in touched {
        if let Ok(Some(project)) = runway_core::deploy::get_project(&ctx.db, &project_id).await {
            let _ =
                runway_core::traefik::update_project_config(&ctx.db, &project, &ctx.settings, &[])
                    .await;
        }
    }
}

/// Keep `remote_node.status`/`updated_at` honest between manual checks:
/// probe every active node with a bounded ping. `waiting` and
/// `disabled` are skipped — approval and enablement own those.
/// jarvis: ceiling — sequential probes, upgrade to a JoinSet if fleets grow
async fn sweep_nodes(ctx: &Ctx) {
    let nodes: Vec<runway_core::models::RemoteNode> =
        match sqlx::query_as("SELECT * FROM remote_node WHERE status IN ('online','offline')")
            .fetch_all(&ctx.db)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "node sweep query failed");
                return;
            }
        };
    for n in nodes {
        let up = match dkr::node_client(&ctx.db, &n.id, &ctx.crypto).await {
            Some(c) => tokio::time::timeout(std::time::Duration::from_secs(10), c.ping())
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false),
            None => false,
        };
        let status = if up { "online" } else { "offline" };
        let _ = sqlx::query("UPDATE remote_node SET status = $2, updated_at = now() WHERE id = $1")
            .bind(&n.id)
            .bind(status)
            .execute(&ctx.db)
            .await;
    }
}
