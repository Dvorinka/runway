//! Monitor loop — port of workers/monitor.py.
//!
//! Polls `deploy`-status deployments: probes the container's HTTP port via
//! the workspace network (the runway container attaches itself), then
//! enqueues finalize on success or fail on exit/timeout. Also sweeps
//! observed_status for completed deployments.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bollard::container::InspectContainerOptions;
use chrono::Utc;
use serde_json::json;

use runway_core::deploy::{self, edge_network_name};
use runway_core::docker as dkr;
use runway_core::models::Deployment;

use crate::Ctx;

/// deployment_id → deadline for readiness probing.
type ProbeState = HashMap<String, chrono::DateTime<Utc>>;
/// deployment_id → last metrics sample (stats calls are not free).
type SampleState = HashMap<String, tokio::time::Instant>;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

pub async fn run(ctx: Ctx) {
    let interval = Duration::from_secs(ctx.settings.monitor_interval_seconds.max(1));
    let mut ticker = tokio::time::interval(interval);
    let mut probe_state: ProbeState = HashMap::new();
    let mut sample_state: SampleState = HashMap::new();
    let mut last_prune = tokio::time::Instant::now();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    loop {
        ticker.tick().await;
        if let Err(e) = tick(&ctx, &http, &mut probe_state, &mut sample_state).await {
            tracing::error!(error = %e, "monitor tick failed");
        }
        // Retention sweep: 24h of 30s metric samples ≈ 2.9k
        // rows/deployment; deployment log files are dropped after 7d.
        if last_prune.elapsed() > Duration::from_secs(3600) {
            last_prune = tokio::time::Instant::now();
            let _ =
                sqlx::query("DELETE FROM deployment_metric WHERE ts < now() - interval '24 hours'")
                    .execute(&ctx.db)
                    .await;
            let removed = ctx.logs.prune_older_than(7).await;
            if removed > 0 {
                tracing::info!(removed, "pruned stale deployment logs");
            }
        }
    }
}

async fn tick(
    ctx: &Ctx,
    http: &reqwest::Client,
    probe_state: &mut ProbeState,
    sample_state: &mut SampleState,
) -> anyhow::Result<()> {
    // Active deployments being brought up.
    let deploying: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment
         WHERE status = 'deploy' AND conclusion IS NULL AND container_id IS NOT NULL",
    )
    .fetch_all(&ctx.db)
    .await?;

    let deploying_ids: HashSet<String> = deploying.iter().map(|d| d.id.clone()).collect();
    probe_state.retain(|id, _| deploying_ids.contains(id));

    let self_id = dkr::self_container_id();
    let mut used_networks: HashSet<String> = HashSet::new();
    // Remote-node docker clients + node rows, resolved lazily per tick.
    let mut node_clients: HashMap<String, bollard::Docker> = HashMap::new();
    let mut node_hosts: HashMap<String, String> = HashMap::new();

    for dep in &deploying {
        let Some(container_id) = dep.container_id.clone() else {
            continue;
        };
        let deadline = probe_state
            .entry(dep.id.clone())
            .or_insert_with(|| {
                Utc::now()
                    + chrono::Duration::seconds(ctx.settings.deployment_timeout_seconds as i64)
            })
            .to_owned();

        // Remote deployments live on their node's daemon.
        let node_client = match dep.remote_node_id.as_deref() {
            Some(nid) => {
                if !node_clients.contains_key(nid) {
                    if let Some(c) = dkr::node_client(&ctx.db, nid).await {
                        node_clients.insert(nid.to_string(), c);
                    }
                }
                if !node_hosts.contains_key(nid) {
                    if let Some(h) = node_host(&ctx.db, nid).await {
                        node_hosts.insert(nid.to_string(), h);
                    }
                }
                node_clients.get(nid)
            }
            None => None,
        };
        let docker = node_client.unwrap_or(&ctx.docker);
        if dep.remote_node_id.is_some() && node_client.is_none() {
            // Node gone/unreachable — don't probe local and misjudge.
            continue;
        }

        let info = match docker
            .inspect_container(&container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(i) => i,
            Err(e) if dkr::is_not_found(&e) => {
                enqueue_fail(ctx, &dep.id, "deploy", "container not found").await;
                continue;
            }
            Err(e) => {
                tracing::warn!(deployment_id = dep.id, error = %e, "inspect failed");
                continue;
            }
        };

        let state = info.state.clone().unwrap_or_default();
        let running = state.running.unwrap_or(false);
        if !running {
            let exit = state.exit_code.unwrap_or(-1);
            ctx.logs
                .info(&dep.id, &format!("Container exited (code {exit})"))
                .await;
            enqueue_fail(
                ctx,
                &dep.id,
                "deploy",
                &format!("container exited (code {exit})"),
            )
            .await;
            continue;
        }

        let url = if let Some(nid) = dep.remote_node_id.as_deref() {
            // Remote: probe the published node port — the same address
            // Traefik will load-balance to. Container IPs on a remote
            // daemon aren't reachable from here.
            let Some(host) = node_hosts.get(nid) else {
                continue;
            };
            let Some(port) = dep.remote_port else {
                continue;
            };
            format!("http://{host}:{port}/")
        } else {
            // Probe on the workspace network (cross-team isolation lives
            // there; edge is traefik-facing only).
            let ws_network = dkr::container_label(&info, "runway.workspace_network");
            let edge = dkr::container_label(&info, "runway.edge_network")
                .unwrap_or_else(|| edge_network_name(&dep.id));
            used_networks.insert(edge);

            if let Some(net) = ws_network.clone() {
                used_networks.insert(net.clone());
                if let Some(self_id) = self_id.as_deref() {
                    let _ = dkr::connect_to_network(&ctx.docker, Some(self_id), Some(&net)).await;
                }
            }

            let networks = info
                .network_settings
                .as_ref()
                .and_then(|ns| ns.networks.as_ref())
                .cloned()
                .unwrap_or_default();
            let ip = ws_network
                .as_ref()
                .and_then(|net| networks.get(net))
                .and_then(|ep| ep.ip_address.clone())
                .filter(|ip| !ip.is_empty());

            let Some(ip) = ip else { continue };
            format!("http://{ip}:{}/", dep.serve_port())
        };
        let ready = http.get(&url).send().await.is_ok();
        if ready {
            ctx.logs.info(&dep.id, "Application is ready").await;
            deploy::enqueue(
                &ctx.db,
                "finalize_deployment",
                json!({ "deployment_id": dep.id }),
                0,
            )
            .await?;
            // Move status forward so we don't enqueue twice before the
            // finalize job runs.
            deploy::update_status(
                &ctx.db,
                &ctx.bus,
                &dep.id,
                Some("finalize"),
                None,
                None,
                None,
            )
            .await?;
            probe_state.remove(&dep.id);
        } else if Utc::now() > deadline {
            ctx.logs
                .info(&dep.id, "Timed out waiting for app readiness")
                .await;
            enqueue_fail(
                ctx,
                &dep.id,
                "deploy",
                &format!(
                    "app did not become ready within {}s",
                    ctx.settings.deployment_timeout_seconds
                ),
            )
            .await;
            probe_state.remove(&dep.id);
        }
    }

    // Detach the probe (us) from workspace networks nothing uses.
    if let Some(self_id) = &self_id {
        detach_from_unused(ctx, self_id, &used_networks).await;
    }

    // Observed-state sweep for running containers (reconcile-lite; the
    // full reconciler runs on a slower cadence in reconcile.rs).
    let running: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment
         WHERE container_status = 'running' AND conclusion = 'succeeded'
           AND container_id IS NOT NULL",
    )
    .fetch_all(&ctx.db)
    .await?;
    for dep in running {
        let Some(cid) = dep.container_id.clone() else {
            continue;
        };
        let docker = match dep.remote_node_id.as_deref() {
            Some(nid) => {
                if !node_clients.contains_key(nid) {
                    if let Some(c) = dkr::node_client(&ctx.db, nid).await {
                        node_clients.insert(nid.to_string(), c);
                    }
                }
                match node_clients.get(nid) {
                    Some(c) => c,
                    None => continue,
                }
            }
            None => &ctx.docker,
        };
        let (observed, exit_code) = match docker
            .inspect_container(&cid, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => {
                let s = info.state.unwrap_or_default();
                let status = if s.running.unwrap_or(false) {
                    "running"
                } else if s.paused.unwrap_or(false) {
                    "paused"
                } else if s.dead.unwrap_or(false) {
                    "dead"
                } else {
                    "exited"
                };
                (status, s.exit_code)
            }
            Err(e) if dkr::is_not_found(&e) => ("not_found", None),
            Err(_) => continue,
        };
        // Notify once on the running → down transition. The previous
        // observed_status gates repeats, so a restart-loop flap can't
        // spam one row per tick.
        let down = match (observed, exit_code) {
            ("dead", _) | ("not_found", _) => true,
            ("exited", c) => c != Some(0),
            _ => false,
        };
        if down && matches!(dep.observed_status.as_deref(), None | Some("running")) {
            notify_crash(ctx, &dep, observed, exit_code.map(|c| c as i32)).await;
        }
        // Reconcile-parity: consecutive 404s accumulate so other
        // consumers can tell "blip" from "gone".
        let missing_count = if observed == "not_found" {
            dep.observed_missing_count + 1
        } else {
            0
        };
        sqlx::query(
            "UPDATE deployment SET observed_status = $1, observed_exit_code = $2,
             observed_at = now(), observed_last_seen_at = now(),
             observed_missing_count = $4 WHERE id = $3",
        )
        .bind(observed)
        .bind(exit_code)
        .bind(&dep.id)
        .bind(missing_count)
        .execute(&ctx.db)
        .await?;

        // Throttled stats sample → deployment_metric for sparklines.
        if observed == "running" {
            let due = sample_state
                .get(&dep.id)
                .map(|t| t.elapsed() >= SAMPLE_INTERVAL)
                .unwrap_or(true);
            if due {
                if let Some(s) = dkr::stats_snapshot(docker, &cid).await {
                    let _ = sqlx::query(
                        "INSERT INTO deployment_metric
                         (deployment_id, cpu_pct, mem_used, net_rx, net_tx, pids)
                         VALUES ($1,$2,$3,$4,$5,$6)",
                    )
                    .bind(&dep.id)
                    .bind(s.cpu_pct)
                    .bind(s.mem_used as i64)
                    .bind(s.net_rx as i64)
                    .bind(s.net_tx as i64)
                    .bind(s.pids as i64)
                    .execute(&ctx.db)
                    .await;
                }
                sample_state.insert(dep.id.clone(), tokio::time::Instant::now());
            }
        } else {
            sample_state.remove(&dep.id);
        }
    }
    Ok(())
}

async fn enqueue_fail(ctx: &Ctx, deployment_id: &str, status: &str, reason: &str) {
    let _ = deploy::enqueue(
        &ctx.db,
        "fail_deployment",
        json!({ "deployment_id": deployment_id, "status": status, "reason": reason }),
        0,
    )
    .await;
    // Guard against double-processing while the job is queued.
    let _ =
        sqlx::query("UPDATE deployment SET status = 'fail' WHERE id = $1 AND conclusion IS NULL")
            .bind(deployment_id)
            .execute(&ctx.db)
            .await;
}

/// Disconnect our own container from workspace networks no deployment uses.
async fn detach_from_unused(ctx: &Ctx, self_id: &str, used: &HashSet<String>) {
    let Ok(info) = ctx
        .docker
        .inspect_container(self_id, None::<InspectContainerOptions>)
        .await
    else {
        return;
    };
    let Some(networks) = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.networks.as_ref())
    else {
        return;
    };
    for name in networks.keys() {
        if name.starts_with(deploy::WORKSPACE_NETWORK_PREFIX) && !used.contains(name) {
            // Only detach when no deployment remains on it.
            match dkr::network_has_deployments(&ctx.docker, name).await {
                Ok(true) => continue,
                _ => {
                    let _ =
                        dkr::disconnect_from_network(&ctx.docker, Some(self_id), Some(name)).await;
                }
            }
        }
    }
}

/// Team notification when a serving container goes down. Best-effort —
/// a lookup failure just skips the notification.
async fn notify_crash(ctx: &Ctx, dep: &Deployment, observed: &str, exit_code: Option<i32>) {
    let project =
        sqlx::query_as::<_, (String, String)>("SELECT name, team_id FROM project WHERE id = $1")
            .bind(&dep.project_id)
            .fetch_optional(&ctx.db)
            .await
            .ok()
            .flatten();
    let Some((name, team_id)) = project else {
        return;
    };
    let reason = match observed {
        "not_found" => "container is missing".to_string(),
        "dead" => "container is dead".to_string(),
        _ => format!("container exited (code {})", exit_code.unwrap_or(-1)),
    };
    runway_core::audit::notify_team(
        &ctx.db,
        &team_id,
        "deployment.crashed",
        &format!("App down: {name}"),
        runway_core::audit::Notify {
            body: Some(&format!("{} — {}", &dep.id[..7.min(dep.id.len())], reason)),
            link: Some(&format!(
                "/projects/{}/deployments/{}",
                dep.project_id, dep.id
            )),
            project_id: Some(&dep.project_id),
            ..Default::default()
        },
    )
    .await;
}

/// Node `host` (the address Traefik/monitor reach) for a remote node.
async fn node_host(db: &sqlx::PgPool, node_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT host FROM remote_node WHERE id = $1")
        .bind(node_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}
