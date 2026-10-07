//! cloudflared container lifecycle — port of devpush `services/tunnel.py`.
//!
//! One cloudflared container per tunnel, attached to the network Traefik
//! lives on so ingress rules can target `http://traefik:80`.

use std::collections::HashMap;

use bollard::container::{
    Config, InspectContainerOptions, NetworkingConfig, StartContainerOptions, StopContainerOptions,
};
use bollard::models::{EndpointSettings, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::Docker;

use crate::docker as dkr;
use crate::error::Result;

/// Pinned — `:latest` is a mutable supply-chain risk (devpush noted).
pub const CLOUDFLARED_IMAGE: &str = "cloudflare/cloudflared:2025.2.0";

/// Where instance tunnel state persists (token is creation-time only).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TunnelState {
    pub tunnel_id: String,
    pub token_enc: String,
}

pub fn tunnel_state_path(data_dir: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(data_dir).join("instance-tunnel.json")
}

/// Persisted instance tunnel state, if present.
pub async fn read_tunnel_state(data_dir: &str) -> Option<TunnelState> {
    let raw = tokio::fs::read_to_string(tunnel_state_path(data_dir))
        .await
        .ok()?;
    serde_json::from_str(&raw).ok()
}

/// Add/remove a hostname on the instance tunnel ingress, preserving the
/// existing rules; the catch-all is appended by `update_tunnel_ingress`.
pub async fn sync_ingress(
    cf: &crate::cloudflare::CloudflareClient,
    account_id: &str,
    tunnel_id: &str,
    add: Option<&str>,
    remove: Option<&str>,
) -> Result<bool> {
    let cfg = cf
        .get_tunnel_config(account_id, tunnel_id)
        .await?
        .unwrap_or_default();
    let mut hosts: Vec<String> = cfg["config"]["ingress"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["hostname"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(r) = remove {
        hosts.retain(|h| h != r);
    }
    if let Some(a) = add {
        if !hosts.iter().any(|h| h == a) {
            hosts.push(a.to_string());
        }
    }
    cf.update_tunnel_ingress(account_id, tunnel_id, &hosts, "http://traefik:80")
        .await
}

/// Create-or-replace + start a cloudflared container. Returns its id.
pub async fn ensure_cloudflared(
    docker: &Docker,
    name: &str,
    tunnel_token: &str,
    network: &str,
) -> Result<String> {
    let body = Config {
        image: Some(CLOUDFLARED_IMAGE.to_string()),
        cmd: Some(vec![
            "tunnel".into(),
            "--no-autoupdate".into(),
            "run".into(),
            "--token".into(),
            tunnel_token.into(),
        ]),
        labels: Some(HashMap::from([
            ("runway.managed".into(), "true".into()),
            ("runway.type".into(), "cloudflared".into()),
            ("runway.name".into(), name.to_string()),
        ])),
        networking_config: Some(NetworkingConfig {
            endpoints_config: HashMap::from([(network.to_string(), EndpointSettings::default())]),
        }),
        host_config: Some(HostConfig {
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::ALWAYS),
                maximum_retry_count: None,
            }),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let id = dkr::create_or_replace_container(docker, name, body).await?;
    docker
        .start_container(&id, None::<StartContainerOptions<String>>)
        .await?;
    Ok(id)
}

/// Stop + remove a cloudflared container by name. Missing is success.
pub async fn stop_cloudflared(docker: &Docker, name: &str) -> Result<()> {
    match docker
        .inspect_container(name, None::<InspectContainerOptions>)
        .await
    {
        Ok(_) => {
            let _ = docker
                .stop_container(name, None::<StopContainerOptions>)
                .await;
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
        Err(e) if dkr::is_not_found(&e) => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Container status string ("running", "exited", "not_found", ...).
pub async fn cloudflared_status(docker: &Docker, name: &str) -> String {
    match docker
        .inspect_container(name, None::<InspectContainerOptions>)
        .await
    {
        Ok(info) => info
            .state
            .and_then(|s| s.status.map(|st| st.to_string()))
            .unwrap_or_else(|| "unknown".into()),
        Err(e) if dkr::is_not_found(&e) => "not_found".into(),
        Err(_) => "error".into(),
    }
}

/// Network Traefik is attached to — where cloudflared must live to
/// reach `http://traefik:80`.
pub async fn traefik_network(docker: &Docker) -> Option<String> {
    let id = dkr::traefik_container_id(docker).await?;
    let info = docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .ok()?;
    let networks = info.network_settings?.networks?;
    // Prefer a non-bridge named network; traefik is always on the
    // compose project network in our stacks.
    networks
        .keys()
        .find(|n| n.as_str() != "bridge")
        .cloned()
        .or_else(|| networks.keys().next().cloned())
}
