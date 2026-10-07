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
    docker_client(&node.docker_url).ok()
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
