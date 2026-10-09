use std::collections::HashMap;

use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, ListContainersOptions};
use bollard::errors::Error as BollardError;
use bollard::models::{ContainerInspectResponse, EndpointSettings};
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions, DisconnectNetworkOptions};
use bollard::Docker;

use crate::config::Settings;

pub fn connect(settings: &Settings) -> Result<Docker> {
    docker_client(&settings.docker_host)
}

pub fn docker_client(docker_host: &str) -> Result<Docker> {
    match docker_host {
        "unix:///var/run/docker.sock" | "" => Docker::connect_with_local_defaults()
            .or_else(|_| Docker::connect_with_socket_defaults())
            .map_err(Into::into),
        s if s.starts_with("tcp://") || s.starts_with("http://") || s.starts_with("https://") => {
            let s = s.replacen("tcp://", "http://", 1);
            let addr = s
                .trim_start_matches("http://")
                .trim_start_matches("https://");
            Docker::connect_with_http(addr, 120, bollard::API_DEFAULT_VERSION).map_err(Into::into)
        }
        s => Docker::connect_with_unix(
            s.trim_start_matches("unix://"),
            120,
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(Into::into),
    }
}

pub fn is_not_found(err: &BollardError) -> bool {
    matches!(
        err,
        BollardError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

/// Inspect a container → `(observed_status, exit_code)`.
/// `Some("not_found")` on 404; `None` when the daemon couldn't be
/// reached (caller should keep the previous observation).
pub async fn inspect_observed(
    docker: &Docker,
    container_id: &str,
) -> Option<(String, Option<i64>)> {
    use bollard::container::InspectContainerOptions;
    match docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
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
            Some((status.to_string(), s.exit_code))
        }
        Err(e) if is_not_found(&e) => Some(("not_found".to_string(), None)),
        Err(_) => None,
    }
}

/// Extract a user-facing reason string from a Docker API error.
pub fn create_error_reason(err: &BollardError) -> String {
    match err {
        BollardError::DockerResponseServerError {
            status_code,
            message,
        } => {
            format!("Docker error ({status_code}): {message}")
        }
        other => format!("Docker error: {other}"),
    }
}

// ---------------------------------------------------------------------------
// networks
// ---------------------------------------------------------------------------

pub async fn network_id(docker: &Docker, name: &str) -> Option<String> {
    docker
        .inspect_network::<String>(name, None)
        .await
        .ok()
        .and_then(|n| n.id)
}

/// Create a bridge network if missing. All runway networks carry the
/// `runway.managed` label plus caller-provided labels.
pub async fn ensure_network(
    docker: &Docker,
    name: &str,
    mut labels: HashMap<String, String>,
) -> Result<()> {
    if network_id(docker, name).await.is_some() {
        return Ok(());
    }
    labels.insert("runway.managed".into(), "true".into());
    docker
        .create_network(CreateNetworkOptions {
            name: name.to_string(),
            driver: "bridge".to_string(),
            check_duplicate: true,
            labels,
            ..Default::default()
        })
        .await?;
    Ok(())
}

/// Remove the network when nothing but Traefik (or nothing) remains
/// attached — mirrors `_remove_deployment_network`.
pub async fn remove_network_if_empty(docker: &Docker, name: &str) -> Result<()> {
    let Some(id) = network_id(docker, name).await else {
        return Ok(());
    };
    let net = docker.inspect_network::<String>(name, None).await?;
    let containers = net.containers.unwrap_or_default();
    let traefik = traefik_container_id(docker).await;
    let removable = containers.keys().all(|cid| Some(cid) == traefik.as_ref());
    if removable {
        docker.remove_network(&id).await.or_else(
            |e| {
                if is_not_found(&e) {
                    Ok(())
                } else {
                    Err(e)
                }
            },
        )?;
    }
    Ok(())
}

/// True when the network has containers other than this process's own
/// container attached.
pub async fn network_has_deployments(docker: &Docker, name: &str) -> Result<bool> {
    let net = docker.inspect_network::<String>(name, None).await?;
    let containers = net.containers.unwrap_or_default();
    let self_id = self_container_id();
    Ok(containers.keys().any(|cid| {
        self_id
            .as_deref()
            .map(|s| !cid.starts_with(s))
            .unwrap_or(true)
    }))
}

// ---------------------------------------------------------------------------
// containers
// ---------------------------------------------------------------------------

/// Label from an inspect response.
pub fn container_label(info: &ContainerInspectResponse, key: &str) -> Option<String> {
    info.config.as_ref()?.labels.as_ref()?.get(key).cloned()
}

/// Id of the container running this process (hostname == short id on
/// Docker), or None outside a container.
pub fn self_container_id() -> Option<String> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some(hostname)
}

/// Id of a running service container (e.g. "traefik") by name match.
pub async fn service_container_id(docker: &Docker, name: &str) -> Option<String> {
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        }))
        .await
        .ok()?;
    for c in containers {
        let names = c.names.clone().unwrap_or_default();
        if names
            .iter()
            .any(|n| n.trim_start_matches('/').contains(name))
        {
            return c.id;
        }
    }
    None
}

/// Id of the running Traefik container.
pub async fn traefik_container_id(docker: &Docker) -> Option<String> {
    let filters = HashMap::from([("label".to_string(), vec!["traefik.enable=true".to_string()])]);
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: false,
            filters,
            ..Default::default()
        }))
        .await
        .ok()?;
    containers.into_iter().next()?.id
}

/// Pull an image, draining the progress stream. Cheap when cached.
pub async fn pull_image(docker: &Docker, image: &str) -> Result<()> {
    use bollard::image::CreateImageOptions;
    use futures::StreamExt;
    let (from, tag) = image.rsplit_once(':').unwrap_or((image, "latest"));
    let mut stream = docker.create_image(
        Some(CreateImageOptions {
            from_image: from.to_string(),
            tag: tag.to_string(),
            ..Default::default()
        }),
        None,
        None,
    );
    while let Some(res) = stream.next().await {
        let info = res?;
        if let Some(err) = info.error {
            anyhow::bail!(err);
        }
    }
    Ok(())
}

/// Create a container, removing any stale container with the same name
/// first (idempotent after a worker retry). Does not start it.
pub async fn create_or_replace_container(
    docker: &Docker,
    name: &str,
    body: Config<String>,
) -> crate::Result<String> {
    match docker.inspect_container(name, None).await {
        Ok(_) => {
            let _ = docker.stop_container(name, None).await;
            let _ = docker
                .remove_container(
                    name,
                    Some(bollard::container::RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
        }
        Err(e) if is_not_found(&e) => {}
        Err(e) => return Err(e.into()),
    }
    let created = docker
        .create_container(
            Some(CreateContainerOptions::<String> {
                name: name.to_string(),
                platform: None,
            }),
            body,
        )
        .await?;
    Ok(created.id)
}

/// Connect a container to a network with optional aliases. No-ops when
/// either side is missing — mirrors the defensive style of the Python
/// helpers.
pub async fn connect_to_network(
    docker: &Docker,
    container: Option<&str>,
    network: Option<&str>,
) -> Result<()> {
    let (Some(container), Some(network)) = (container, network) else {
        return Ok(());
    };
    docker
        .connect_network(
            network,
            ConnectNetworkOptions {
                container: container.to_string(),
                endpoint_config: EndpointSettings::default(),
            },
        )
        .await
        .or_else(|e| if is_not_found(&e) { Ok(()) } else { Err(e) })?;
    Ok(())
}

/// Disconnect a container from a network (force). No-ops on missing
/// args or a 404.
pub async fn disconnect_from_network(
    docker: &Docker,
    container: Option<&str>,
    network: Option<&str>,
) -> Result<()> {
    let (Some(container), Some(network)) = (container, network) else {
        return Ok(());
    };
    docker
        .disconnect_network(
            network,
            DisconnectNetworkOptions {
                container: container.to_string(),
                force: true,
            },
        )
        .await
        .or_else(|e| if is_not_found(&e) { Ok(()) } else { Err(e) })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// remote nodes
// ---------------------------------------------------------------------------

/// Host port range published on remote nodes for app traffic.
/// Traefik load-balances to `node.host:port`.
pub const REMOTE_PORT_START: i32 = 49152;
pub const REMOTE_PORT_END: i32 = 49651;

/// Docker client for a remote node — `None` when the node is gone,
/// disabled, or unreachable at connect time.
pub async fn node_client(db: &sqlx::PgPool, node_id: &str) -> Option<Docker> {
    let node: crate::models::RemoteNode = sqlx::query_as("SELECT * FROM remote_node WHERE id = $1")
        .bind(node_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()?;
    if node.status != "online" || node.is_local() {
        return None;
    }
    node_docker_client(&node).ok()
}

/// Client for one node honoring its stored mTLS material: all three PEMs
/// present → `connect_with_ssl`, otherwise plain `docker_client`.
pub fn node_docker_client(node: &crate::models::RemoteNode) -> Result<Docker> {
    let (Some(ca), Some(cert), Some(key)) = (&node.tls_ca, &node.tls_cert, &node.tls_key) else {
        return docker_client(&node.docker_url);
    };
    // bollard's ssl feature is providerless: install ring once or
    // `ClientConfig::builder()` panics on first connect.
    static CRYPTO: std::sync::Once = std::sync::Once::new();
    CRYPTO.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    let dir = std::env::temp_dir().join(format!("runway-node-tls-{}", node.id));
    std::fs::create_dir_all(&dir)?;
    write_pem(&dir.join("ca.pem"), ca)?;
    write_pem(&dir.join("cert.pem"), cert)?;
    write_pem(&dir.join("key.pem"), key)?;
    Docker::connect_with_ssl(
        &node.docker_url,
        &dir.join("key.pem"),
        &dir.join("cert.pem"),
        &dir.join("ca.pem"),
        120,
        bollard::API_DEFAULT_VERSION,
    )
    .map_err(Into::into)
}

/// Write PEM material with owner-only permissions (key files are
/// regenerated from the DB on each call, so contents stay in sync).
#[cfg(unix)]
fn write_pem(path: &std::path::Path, pem: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(pem.as_bytes())?;
    Ok(())
}

/// First host port in `REMOTE_PORT_START..=REMOTE_PORT_END` not
/// published by any container on this daemon.
pub async fn alloc_remote_port(docker: &Docker) -> anyhow::Result<i32> {
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await?;
    let used: std::collections::HashSet<u16> = containers
        .iter()
        .flat_map(|c| c.ports.clone().unwrap_or_default())
        .filter_map(|p| p.public_port)
        .collect();
    (REMOTE_PORT_START..=REMOTE_PORT_END)
        .find(|p| !used.contains(&(*p as u16)))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote node: no free publish port in {REMOTE_PORT_START}-{REMOTE_PORT_END}"
            )
        })
}

/// One-shot resource snapshot for a running container.
#[derive(Debug, Clone, Copy)]
pub struct StatsSnapshot {
    pub cpu_pct: f64,
    pub mem_used: u64,
    pub mem_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub pids: u64,
}

/// `docker stats --no-stream` equivalent. None when the container is
/// gone or the stats call fails/times out (8s).
pub async fn stats_snapshot(docker: &Docker, container_id: &str) -> Option<StatsSnapshot> {
    use bollard::container::{MemoryStatsStats, StatsOptions};
    use futures::StreamExt;

    let mut stream = docker.stats(
        container_id,
        Some(StatsOptions {
            stream: false,
            one_shot: true,
        }),
    );
    let frame = tokio::time::timeout(std::time::Duration::from_secs(8), stream.next())
        .await
        .ok()
        .flatten()?;
    let s = frame.ok()?;

    let cpu_delta = s
        .cpu_stats
        .cpu_usage
        .total_usage
        .saturating_sub(s.precpu_stats.cpu_usage.total_usage);
    let sys_delta = s
        .cpu_stats
        .system_cpu_usage
        .unwrap_or(0)
        .saturating_sub(s.precpu_stats.system_cpu_usage.unwrap_or(0));
    let ncpu = s
        .cpu_stats
        .online_cpus
        .or_else(|| {
            s.cpu_stats
                .cpu_usage
                .percpu_usage
                .as_ref()
                .map(|p| p.len() as u64)
        })
        .unwrap_or(1)
        .max(1);
    let cpu_pct = if sys_delta > 0 {
        (cpu_delta as f64 / sys_delta as f64) * ncpu as f64 * 100.0
    } else {
        0.0
    };
    // Page cache counts toward cgroup usage — v1 calls it `cache`,
    // v2 `inactive_file`. Subtract so the number reads like RSS.
    let cache = match s.memory_stats.stats {
        Some(MemoryStatsStats::V1(v1)) => v1.cache,
        Some(MemoryStatsStats::V2(v2)) => v2.inactive_file,
        None => 0,
    };
    let mut rx = 0u64;
    let mut tx = 0u64;
    if let Some(nets) = &s.networks {
        for n in nets.values() {
            rx += n.rx_bytes;
            tx += n.tx_bytes;
        }
    }
    Some(StatsSnapshot {
        cpu_pct,
        mem_used: s.memory_stats.usage.unwrap_or(0).saturating_sub(cache),
        mem_limit: s.memory_stats.limit.unwrap_or(0),
        net_rx: rx,
        net_tx: tx,
        pids: s.pids_stats.current.unwrap_or(0),
    })
}
