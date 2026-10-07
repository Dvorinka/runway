//! Instance-level Cloudflare Tunnel — the CGNAT story.
//!
//! Auto-enqueued as `ensure_instance_tunnel` at `runway serve` when
//! CF_API_TOKEN + CF_ACCOUNT_ID are set. Steps: find-or-create the
//! remotely-managed tunnel, push ingress (APP_HOSTNAME +
//! *.DEPLOY_DOMAIN → traefik), create the CNAMEs, then run a
//! cloudflared container on Traefik's network.

use runway_core::cloudflare::CloudflareClient;
use runway_core::tunnel::{self as cf_tunnel, TunnelState};

use crate::Ctx;

const TUNNEL_NAME: &str = "runway-instance";
const CONTAINER_NAME: &str = "cloudflared-instance";

/// `ensure_instance_tunnel` job.
pub async fn ensure_instance(ctx: &Ctx) -> anyhow::Result<()> {
    let (Some(api_token), Some(account_id)) = (
        ctx.settings.cf_api_token.clone(),
        ctx.settings.cf_account_id.clone(),
    ) else {
        anyhow::bail!("cloudflare not configured (CF_API_TOKEN/CF_ACCOUNT_ID)");
    };
    let cf = CloudflareClient::new(api_token);

    // 1. Find-or-create. The token is only returned at creation, so a
    //    persisted state whose tunnel vanished upstream is rebuilt.
    let mut state = cf_tunnel::read_tunnel_state(&ctx.settings.data_dir).await;
    if let Some(s) = &state {
        if cf.get_tunnel(&account_id, &s.tunnel_id).await?.is_none() {
            tracing::warn!(
                tunnel_id = s.tunnel_id,
                "instance tunnel gone upstream — recreating"
            );
            state = None;
        }
    }
    if state.is_none() {
        let t = cf
            .create_tunnel(&account_id, TUNNEL_NAME)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cloudflare returned no tunnel token"))?;
        state = Some(TunnelState {
            tunnel_id: t.id,
            token_enc: ctx.crypto.encrypt(&t.token)?,
        });
        let path = cf_tunnel::tunnel_state_path(&ctx.settings.data_dir);
        tokio::fs::write(&path, serde_json::to_string(state.as_ref().unwrap())?).await?;
    }
    let state = state.expect("state populated");
    tracing::info!(tunnel_id = state.tunnel_id, "instance tunnel resolved");

    // 2. Ingress — keep any hostnames added later (custom domains) by
    //    merging rather than overwriting.
    let hostnames = vec![
        ctx.settings.app_hostname.clone(),
        format!("*.{}", ctx.settings.deploy_domain),
    ];
    for hostname in &hostnames {
        let ok = cf_tunnel::sync_ingress(&cf, &account_id, &state.tunnel_id, Some(hostname), None)
            .await?;
        anyhow::ensure!(ok, "tunnel ingress update rejected by cloudflare");
    }

    // 3. DNS — CNAME app + wildcard to the tunnel endpoint, proxied.
    let target = format!("{}.cfargotunnel.com", state.tunnel_id);
    for hostname in &hostnames {
        cf.create_or_update_dns_record(hostname, &target, true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no cloudflare zone covers {hostname}"))?;
    }

    // 4. cloudflared on Traefik's network so ingress hits traefik:80.
    let network = cf_tunnel::traefik_network(&ctx.docker)
        .await
        .ok_or_else(|| anyhow::anyhow!("traefik container not found — is the stack up?"))?;
    let token = ctx.crypto.decrypt(&state.token_enc)?;
    cf_tunnel::ensure_cloudflared(&ctx.docker, CONTAINER_NAME, &token, &network).await?;
    tracing::info!("cloudflared instance container running ({CONTAINER_NAME})");
    Ok(())
}

/// `teardown_instance_tunnel` job — remove container, keep the tunnel
/// record upstream (idempotent teardown without orphaning CF state).
pub async fn teardown_instance(ctx: &Ctx) -> anyhow::Result<()> {
    cf_tunnel::stop_cloudflared(&ctx.docker, CONTAINER_NAME).await?;
    Ok(())
}
