//! Deploy pipeline — port of workers/tasks/deployment.py.
//!
//! start:      prepare → build commands → create container → deploy
//! finalize:   aliases + traefik config → completed/succeeded
//! fail:       stop + schedule delete + network cleanup → completed/failed
//! delete_container / cleanup_inactive / reconcile_edge_network: janitors.

use std::collections::HashMap;

use bollard::container::{
    Config as ContainerConfig, InspectContainerOptions, ListContainersOptions, LogsOptions,
    NetworkingConfig, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::{EndpointSettings, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::volume::CreateVolumeOptions;
use futures::StreamExt;
use serde_json::json;

use runway_core::deploy::{self, edge_network_name, workspace_network_name};
use runway_core::docker as dkr;
use runway_core::models::{Deployment, Project};
use runway_core::presets;

use crate::Ctx;

/// `start_deployment` job — clone, build, run.
pub async fn start(ctx: &Ctx, deployment_id: &str) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        anyhow::bail!("deployment {deployment_id} not found");
    };
    if deployment.conclusion.as_deref() == Some("canceled") {
        return Ok(());
    }
    let Some(project) = deploy::get_project(&ctx.db, &deployment.project_id).await? else {
        anyhow::bail!("project {} not found", deployment.project_id);
    };

    deploy::update_status(
        &ctx.db,
        &ctx.bus,
        deployment_id,
        Some("prepare"),
        None,
        None,
        None,
    )
    .await?;

    let result = run_pipeline(ctx, &deployment, &project).await;
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            deploy::enqueue(
                &ctx.db,
                "fail_deployment",
                json!({
                    "deployment_id": deployment_id,
                    "status": "deploy",
                    "reason": format!("Deployment failed unexpectedly: {e}"),
                }),
                0,
            )
            .await?;
            Err(e)
        }
    }
}

async fn run_pipeline(ctx: &Ctx, deployment: &Deployment, project: &Project) -> anyhow::Result<()> {
    let deployment_id = &deployment.id;
    let docker = &ctx.docker;
    let log = |msg: &str| {
        let logs = ctx.logs.clone();
        let id = deployment_id.clone();
        let msg = msg.to_string();
        async move { logs.info(&id, &msg).await }
    };

    // -- Env vars ------------------------------------------------------
    let mut env: Vec<String> =
        deploy::runtime_env_vars(deployment, project, &ctx.settings, &ctx.crypto)?
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

    // -- Dependency cache volume ---------------------------------------
    let mut binds: Vec<String> = vec![];
    let cache_volume = format!("runway-cache-{}", &project.id[..12.min(project.id.len())]);
    let _ = docker
        .create_volume(CreateVolumeOptions {
            name: cache_volume.clone(),
            labels: HashMap::from([
                ("runway.project_id".into(), project.id.clone()),
                ("runway.cache".into(), "dependencies".into()),
            ]),
            ..Default::default()
        })
        .await;
    for path in [
        "/root/.npm",
        "/root/.cache/pip",
        "/root/.bun/install/cache",
        "/go/pkg/mod",
        "/root/.composer/cache",
        "/root/.cargo/registry",
    ] {
        binds.push(format!("{cache_volume}:{path}"));
    }

    // -- Commands ------------------------------------------------------
    let mut commands: Vec<String> = vec![format!(
        "echo 'Cloning {} (Branch: {}, Commit: {})'",
        deployment.repo_full_name,
        deployment.branch,
        &deployment.commit_sha[..7.min(deployment.commit_sha.len())]
    )];

    match deployment.repo_provider.as_str() {
        "github" | "github_enterprise" => {
            let Some(github) = ctx.github.as_ref() else {
                anyhow::bail!("GitHub App not configured");
            };
            let installation_id = project
                .github_installation_id
                .ok_or_else(|| anyhow::anyhow!("project has no GitHub installation"))?;
            let token = github
                .installation_token(&ctx.db, &ctx.crypto, installation_id)
                .await?;
            env.push(format!("RUNWAY_GITHUB_TOKEN={token}"));
            commands.push(format!(
                "git init -q && \
                 printf '%s\\n' '#!/bin/sh' \
                 'case \"$1\" in *Username*) echo \"x-access-token\";; *) echo \"$RUNWAY_GITHUB_TOKEN\";; esac' \
                 > /tmp/runway-git-askpass && \
                 chmod 700 /tmp/runway-git-askpass && \
                 export GIT_ASKPASS=/tmp/runway-git-askpass GIT_TERMINAL_PROMPT=0 && \
                 git fetch -q --depth 1 https://github.com/{repo}.git {sha} && \
                 git checkout -q FETCH_HEAD && \
                 unset GIT_ASKPASS GIT_TERMINAL_PROMPT RUNWAY_GITHUB_TOKEN && \
                 rm -f /tmp/runway-git-askpass",
                repo = deployment.repo_full_name,
                sha = deployment.commit_sha,
            ));
        }
        other => anyhow::bail!("repo provider '{other}' not supported yet"),
    }

    let config = &deployment.config;
    let root_dir = config
        .get("root_directory")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string();
    if !root_dir.is_empty() {
        commands.push(format!("echo 'Changing root directory to {root_dir}'"));
        commands.push(format!(
            "test -d {root_dir} || {{ printf '\\033[31mError: root directory %s not found\\033[0m\\n' {root_dir} 1>&2; exit 1; }}"
        ));
        commands.push(format!("cd {root_dir}"));
    }

    // Build / pre-deploy / start — runway.json overrides via jq in-container.
    let cfg_str = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("");
    if !cfg_str("build_command").is_empty() {
        commands.push("echo 'Installing dependencies...'".into());
        commands.push(config_command("build_command", cfg_str("build_command")));
    }
    let pre_deploy = cfg_str("pre_deploy_command");
    commands.push(format!(
        "if [ -f runway.json ] && command -v jq >/dev/null 2>&1; then \
         OVERRIDE=$(jq -r '.pre_deploy_command // empty' runway.json); \
         if [ -n \"$OVERRIDE\" ]; then \
         echo 'Running pre-deploy command from runway.json...'; \
         ( $OVERRIDE ); \
         elif [ -n '{pre_deploy}' ]; then \
         echo 'Running pre-deploy command...'; \
         ( {pre_deploy} ); \
         fi; else \
         if [ -n '{pre_deploy}' ]; then \
         echo 'Running pre-deploy command...'; \
         ( {pre_deploy} ); \
         fi; fi"
    ));
    commands.push("echo 'Starting application...'".into());
    commands.push(config_command("start_command", cfg_str("start_command")));

    // -- Networks ------------------------------------------------------
    let edge_network = edge_network_name(deployment_id);
    let workspace_network = workspace_network_name(&project.team_id);
    dkr::ensure_network(
        docker,
        &edge_network,
        HashMap::from([
            ("runway.network_role".into(), "edge".into()),
            ("runway.deployment_id".into(), deployment_id.clone()),
        ]),
    )
    .await?;
    dkr::ensure_network(
        docker,
        &workspace_network,
        HashMap::from([
            ("runway.network_role".into(), "workspace".into()),
            ("runway.workspace_id".into(), project.team_id.clone()),
        ]),
    )
    .await?;

    // -- Labels --------------------------------------------------------
    let app_port = deployment.deployment_port();
    let router = format!("deployment-{deployment_id}");
    let project_slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let mut labels = HashMap::from([
        ("traefik.enable".into(), "true".into()),
        (
            format!("traefik.http.routers.{router}.rule"),
            format!(
                "Host(`{}.{}`)",
                deployment.slug(&project_slug),
                ctx.settings.deploy_domain
            ),
        ),
        (
            format!("traefik.http.routers.{router}.service"),
            format!("{router}@docker"),
        ),
        (
            format!("traefik.http.routers.{router}.priority"),
            "10".into(),
        ),
        (
            format!("traefik.http.services.{router}.loadbalancer.server.port"),
            app_port.to_string(),
        ),
        ("traefik.docker.network".into(), edge_network.clone()),
        ("runway.deployment_id".into(), deployment_id.clone()),
        ("runway.project_id".into(), project.id.clone()),
        ("runway.team_id".into(), project.team_id.clone()),
        (
            "runway.environment_id".into(),
            deployment.environment_id.clone(),
        ),
        ("runway.branch".into(), deployment.branch.clone()),
        ("runway.edge_network".into(), edge_network.clone()),
        ("runway.workspace_network".into(), workspace_network.clone()),
    ]);
    if ctx.settings.url_scheme == "https" {
        labels.insert(
            format!("traefik.http.routers.{router}.entrypoints"),
            "websecure".into(),
        );
        labels.insert(format!("traefik.http.routers.{router}.tls"), "true".into());
        labels.insert(
            format!("traefik.http.routers.{router}.tls.certresolver"),
            "le".into(),
        );
    } else {
        labels.insert(
            format!("traefik.http.routers.{router}.entrypoints"),
            "web".into(),
        );
    }

    // -- Resource limits -----------------------------------------------
    let mut cpus = ctx.settings.default_cpus;
    let mut memory_mb = ctx.settings.default_memory_mb;
    if ctx.settings.allow_custom_cpu {
        if let Some(override_cpus) = config.get("cpus").and_then(|v| v.as_f64()) {
            if override_cpus > 0.0 {
                cpus = f64::min(override_cpus, ctx.settings.max_cpus);
            }
        }
    }
    if ctx.settings.allow_custom_memory {
        if let Some(override_mem) = config.get("memory").and_then(|v| v.as_i64()) {
            if override_mem > 0 {
                memory_mb = i64::min(override_mem, ctx.settings.max_memory_mb);
            }
        }
    }

    // -- Image ---------------------------------------------------------
    let mut runner_image = deployment.image.clone();
    if runner_image.is_none() {
        if let Some(img) = config.get("override_image").and_then(|v| v.as_str()) {
            runner_image = Some(img.to_string());
        } else {
            let slug = config
                .get("runner")
                .or_else(|| config.get("image"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("runner not set in deployment config"))?;
            runner_image = presets::runner_image(slug).map(str::to_string);
        }
    }
    let Some(runner_image) = runner_image else {
        anyhow::bail!("runner image not found for deployment");
    };

    if config
        .get("dockerfile_path")
        .and_then(|v| v.as_str())
        .is_some()
    {
        anyhow::bail!(
            "dockerfile_path builds are not supported yet — use override_image or a preset"
        );
    }

    log("Checking runner image availability...").await;
    if docker.inspect_image(&runner_image).await.is_err() {
        log(&format!("Pulling runner image ({runner_image})...")).await;
        pull_image(docker, &runner_image).await?;
        log("Runner image pulled").await;
    } else {
        log(&format!("Runner image already present ({runner_image})")).await;
    }

    // -- Container -----------------------------------------------------
    log("Preparing and starting container...").await;
    let container_name = format!("runner-{}", &deployment_id[..7.min(deployment_id.len())]);

    let restart_policy = RestartPolicy {
        name: match ctx.settings.deployment_restart_policy.as_str() {
            "always" => Some(RestartPolicyNameEnum::ALWAYS),
            "unless-stopped" => Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            "on-failure" => Some(RestartPolicyNameEnum::ON_FAILURE),
            _ => Some(RestartPolicyNameEnum::NO),
        },
        maximum_retry_count: if ctx.settings.deployment_restart_policy == "on-failure" {
            Some(ctx.settings.deployment_restart_max_retries)
        } else {
            None
        },
    };

    let mut host_config = HostConfig {
        security_opt: Some(vec!["no-new-privileges:true".into()]),
        restart_policy: Some(restart_policy),
        log_config: Some(bollard::models::HostConfigLogConfig {
            typ: Some("json-file".into()),
            config: Some(HashMap::from([
                ("max-size".into(), "10m".into()),
                ("max-file".into(), "5".into()),
            ])),
        }),
        ..Default::default()
    };
    if cpus > 0.0 {
        host_config.nano_cpus = Some((cpus * 1e9) as i64);
    }
    if memory_mb > 0 {
        host_config.memory = Some(memory_mb * 1024 * 1024);
    }
    if !binds.is_empty() {
        host_config.binds = Some(binds);
    }

    let entrypoint = config.get("entrypoint").and_then(|v| v.as_str());
    let cmd = match entrypoint {
        Some(ep) => vec!["sh".to_string(), "-c".to_string(), ep.to_string()],
        None => vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            commands.join(" && "),
        ],
    };

    let body = ContainerConfig {
        image: Some(runner_image),
        env: Some(env),
        working_dir: Some("/app".into()),
        labels: Some(labels),
        networking_config: Some(NetworkingConfig {
            endpoints_config: HashMap::from([
                (edge_network.clone(), EndpointSettings::default()),
                (workspace_network.clone(), EndpointSettings::default()),
            ]),
        }),
        host_config: Some(host_config),
        cmd: Some(cmd),
        ..Default::default()
    };

    let container_id = match dkr::create_or_replace_container(docker, &container_name, body).await {
        Ok(id) => id,
        Err(runway_core::Error::Docker(e)) => {
            let reason = dkr::create_error_reason(&e);
            deploy::enqueue(
                &ctx.db,
                "fail_deployment",
                json!({
                    "deployment_id": deployment_id,
                    "status": "prepare",
                    "reason": reason,
                }),
                0,
            )
            .await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    docker
        .start_container(&container_id, None::<StartContainerOptions<String>>)
        .await?;

    sqlx::query("UPDATE deployment SET container_id = $1 WHERE id = $2")
        .bind(&container_id)
        .bind(deployment_id)
        .execute(&ctx.db)
        .await?;
    deploy::update_status(
        &ctx.db,
        &ctx.bus,
        deployment_id,
        Some("deploy"),
        None,
        None,
        Some("running"),
    )
    .await?;

    deploy::enqueue(
        &ctx.db,
        "reconcile_edge_network",
        json!({ "deployment_id": deployment_id }),
        0,
    )
    .await?;

    spawn_log_tailer(ctx, container_id.clone(), deployment_id.clone());
    Ok(())
}

/// `if [ -f runway.json ] && jq ...` wrapper for a config-command step.
fn config_command(key: &str, command: &str) -> String {
    format!(
        "if [ -f runway.json ] && command -v jq >/dev/null 2>&1; then \
         OVERRIDE=$(jq -r '.{key} // empty' runway.json); \
         if [ -n \"$OVERRIDE\" ]; then \
         echo 'Using {key} from runway.json'; \
         ( $OVERRIDE ); \
         else \
         ( {command} ); \
         fi; else \
         ( {command} ); \
         fi"
    )
}

async fn pull_image(docker: &bollard::Docker, image: &str) -> anyhow::Result<()> {
    use bollard::image::CreateImageOptions;
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

/// Tail `docker logs -f` into the deployment log file + SSE bus.
fn spawn_log_tailer(ctx: &Ctx, container_id: String, deployment_id: String) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut stream = ctx.docker.logs::<String>(
            &container_id,
            Some(LogsOptions {
                follow: true,
                stdout: true,
                stderr: true,
                timestamps: false,
                tail: "0".into(),
                ..Default::default()
            }),
        );
        while let Some(item) = stream.next().await {
            match item {
                Ok(output) => {
                    let text = output.to_string();
                    for line in text.lines() {
                        ctx.logs.append_raw(&deployment_id, "runtime", line).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(deployment_id, error = %e, "log tail ended");
                    break;
                }
            }
        }
    });
}

/// `finalize_deployment` — aliases + traefik + mark succeeded.
pub async fn finalize(ctx: &Ctx, deployment_id: &str) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        anyhow::bail!("deployment {deployment_id} not found");
    };
    if deployment.conclusion.as_deref() == Some("canceled") {
        return Ok(());
    }
    let Some(project) = deploy::get_project(&ctx.db, &deployment.project_id).await? else {
        anyhow::bail!("project not found");
    };

    if let Err(e) = async {
        deploy::setup_aliases(&ctx.db, &deployment, &project, &ctx.settings).await?;
        runway_core::traefik::update_project_config(
            &ctx.db,
            &project,
            &ctx.settings,
            std::slice::from_ref(&deployment.id),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await
    {
        deploy::enqueue(
            &ctx.db,
            "fail_deployment",
            json!({
                "deployment_id": deployment_id,
                "status": "finalize",
                "reason": "Failed to finalize deployment (aliases/routing). The app may still be running.",
            }),
            0,
        )
        .await?;
        return Err(e);
    }

    deploy::update_status(
        &ctx.db,
        &ctx.bus,
        deployment_id,
        Some("completed"),
        Some("succeeded"),
        None,
        None,
    )
    .await?;
    ctx.logs.info(deployment_id, "Deployment succeeded").await;

    deploy::enqueue(
        &ctx.db,
        "cleanup_inactive_containers",
        json!({ "project_id": deployment.project_id }),
        0,
    )
    .await?;
    Ok(())
}

/// `fail_deployment` — stop container, schedule deletion, mark failed.
pub async fn fail(
    ctx: &Ctx,
    deployment_id: &str,
    status: &str,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        anyhow::bail!("deployment {deployment_id} not found");
    };
    if deployment.conclusion.is_some() {
        return Ok(());
    }

    deploy::update_status(
        &ctx.db,
        &ctx.bus,
        deployment_id,
        Some("fail"),
        None,
        None,
        None,
    )
    .await?;
    ctx.logs
        .info(
            deployment_id,
            &format!("Error: {}", reason.unwrap_or("deployment failed")),
        )
        .await;

    if let Some(container_id) = &deployment.container_id {
        if !["removed", "stopped"].contains(&deployment.container_status.as_deref().unwrap_or("")) {
            match ctx
                .docker
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await
            {
                Ok(info) => {
                    let _ = ctx
                        .docker
                        .stop_container(container_id, None::<StopContainerOptions>)
                        .await;
                    deploy::enqueue(
                        &ctx.db,
                        "delete_container",
                        json!({ "deployment_id": deployment_id }),
                        ctx.settings.container_delete_grace_seconds as i64,
                    )
                    .await?;
                    deploy::update_status(
                        &ctx.db,
                        &ctx.bus,
                        deployment_id,
                        None,
                        None,
                        None,
                        Some("stopped"),
                    )
                    .await?;
                    let _ = info;
                }
                Err(e) if dkr::is_not_found(&e) => {
                    deploy::update_status(
                        &ctx.db,
                        &ctx.bus,
                        deployment_id,
                        None,
                        None,
                        None,
                        Some("removed"),
                    )
                    .await?;
                    deploy::cleanup_edge_network(&ctx.docker, deployment_id).await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        deploy::cleanup_edge_network(&ctx.docker, deployment_id).await?;
    }

    deploy::update_status(
        &ctx.db,
        &ctx.bus,
        deployment_id,
        Some("completed"),
        Some("failed"),
        Some(json!({ "status": status, "message": reason.unwrap_or("Deployment failed") })),
        None,
    )
    .await?;
    Ok(())
}

/// `delete_container` — remove a stopped deployment container + edge network.
pub async fn delete_container(ctx: &Ctx, deployment_id: &str) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        return Ok(());
    };
    let Some(container_id) = &deployment.container_id else {
        return Ok(());
    };

    match ctx
        .docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
    {
        Ok(info) => {
            let edge = dkr::container_label(&info, "runway.edge_network")
                .unwrap_or_else(|| edge_network_name(deployment_id));
            let _ = ctx
                .docker
                .stop_container(container_id, None::<StopContainerOptions>)
                .await;
            ctx.docker
                .remove_container(
                    container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await?;
            sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                .bind(deployment_id)
                .execute(&ctx.db)
                .await?;
            let traefik_id = dkr::service_container_id(&ctx.docker, "traefik").await;
            dkr::disconnect_from_network(&ctx.docker, traefik_id.as_deref(), Some(&edge)).await?;
            let _ = dkr::remove_network_if_empty(&ctx.docker, &edge).await;
        }
        Err(e) if dkr::is_not_found(&e) => {
            sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                .bind(deployment_id)
                .execute(&ctx.db)
                .await?;
            deploy::cleanup_edge_network(&ctx.docker, deployment_id).await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// `cleanup_inactive_containers` — stop/remove containers no alias points at.
pub async fn cleanup_inactive(ctx: &Ctx, project_id: &str) -> anyhow::Result<()> {
    let Some(project) = deploy::get_project(&ctx.db, project_id).await? else {
        return Ok(());
    };
    if project.status == "deleted" {
        return Ok(());
    }

    let active: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT a.deployment_id FROM alias a
         JOIN deployment d ON a.deployment_id = d.id
         WHERE d.project_id = $1
         UNION
         SELECT a.previous_deployment_id FROM alias a
         JOIN deployment d ON a.previous_deployment_id = d.id
         WHERE d.project_id = $1",
    )
    .bind(project_id)
    .fetch_all(&ctx.db)
    .await?;
    let active_ids: std::collections::HashSet<&str> =
        active.iter().filter_map(|(id,)| id.as_deref()).collect();

    let inactive: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment
         WHERE project_id = $1 AND container_id IS NOT NULL
           AND container_status = 'running' AND status = 'completed'",
    )
    .bind(project_id)
    .fetch_all(&ctx.db)
    .await?;

    for dep in inactive
        .iter()
        .filter(|d| !active_ids.contains(d.id.as_str()))
    {
        let Some(container_id) = dep.container_id.clone() else {
            continue;
        };
        match ctx
            .docker
            .inspect_container(&container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => {
                let _ = ctx
                    .docker
                    .stop_container(&container_id, None::<StopContainerOptions>)
                    .await;
                let _ = ctx
                    .docker
                    .remove_container(
                        &container_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
                sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                    .bind(&dep.id)
                    .execute(&ctx.db)
                    .await?;
                let edge = dkr::container_label(&info, "runway.edge_network")
                    .unwrap_or_else(|| edge_network_name(&dep.id));
                let traefik_id = dkr::service_container_id(&ctx.docker, "traefik").await;
                dkr::disconnect_from_network(&ctx.docker, traefik_id.as_deref(), Some(&edge))
                    .await?;
                let _ = dkr::remove_network_if_empty(&ctx.docker, &edge).await;
            }
            Err(e) if dkr::is_not_found(&e) => {
                sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                    .bind(&dep.id)
                    .execute(&ctx.db)
                    .await?;
            }
            Err(e) => {
                tracing::warn!(deployment_id = dep.id, error = %e, "cleanup failed for container")
            }
        }
    }
    Ok(())
}

/// `reconcile_edge_network` — attach Traefik to edge networks that host
/// deployment containers (needed because edge networks are dynamic).
pub async fn reconcile_edge_network(
    ctx: &Ctx,
    deployment_id: Option<String>,
) -> anyhow::Result<()> {
    let mut edge_networks: std::collections::BTreeSet<String> = Default::default();

    if let Some(id) = deployment_id {
        if let Some(dep) = deploy::get(&ctx.db, &id).await? {
            if let Some(cid) = &dep.container_id {
                if let Ok(info) = ctx
                    .docker
                    .inspect_container(cid, None::<InspectContainerOptions>)
                    .await
                {
                    if let Some(edge) = dkr::container_label(&info, "runway.edge_network") {
                        edge_networks.insert(edge);
                    }
                }
            }
        }
    }

    if edge_networks.is_empty() {
        // Startup pass: scan all containers for the label.
        let containers = ctx
            .docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await?;
        for c in containers {
            let labels = c.labels.unwrap_or_default();
            if let Some(edge) = labels.get("runway.edge_network") {
                edge_networks.insert(edge.clone());
            }
        }
    }

    let Some(traefik_id) = dkr::service_container_id(&ctx.docker, "traefik").await else {
        tracing::warn!("traefik container not found; skipping edge reconcile");
        return Ok(());
    };

    for edge in edge_networks {
        dkr::connect_to_network(&ctx.docker, Some(&traefik_id), Some(&edge)).await?;
    }
    Ok(())
}
