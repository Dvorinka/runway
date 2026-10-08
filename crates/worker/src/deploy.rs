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

    // Remote node: devpush `get_docker_url_for_project` — an online
    // non-local node gets its own daemon, anything else falls back to
    // the local socket. Unlike devpush we have a return path: the serve
    // container publishes a host port on the node and the Traefik file
    // config load-balances to `node.host:port`.
    let mut remote_node: Option<runway_core::models::RemoteNode> = None;
    let node_client = if let Some(nid) = project.remote_node_id.as_deref() {
        let node: Option<runway_core::models::RemoteNode> =
            sqlx::query_as("SELECT * FROM remote_node WHERE id = $1")
                .bind(nid)
                .fetch_optional(&ctx.db)
                .await?;
        match node {
            Some(n) if n.status == "online" && !n.is_local() => {
                let client = runway_core::docker::docker_client(&n.docker_url)?;
                tracing::info!(node = %nid, name = %n.name, "deploying on remote node");
                remote_node = Some(n);
                Some(client)
            }
            Some(n) => {
                tracing::warn!(node = %nid, status = %n.status,
                    "remote node unavailable, falling back to local daemon");
                None
            }
            None => None,
        }
    } else {
        None
    };
    let docker = node_client.as_ref().unwrap_or(&ctx.docker);

    // Persist the node + allocated publish port early so the monitor and
    // teardown resolve the right daemon even if the pipeline fails later.
    let remote_port = if let Some(node) = &remote_node {
        let port = runway_core::docker::alloc_remote_port(docker).await?;
        sqlx::query("UPDATE deployment SET remote_node_id = $1, remote_port = $2 WHERE id = $3")
            .bind(&node.id)
            .bind(port)
            .bind(deployment_id)
            .execute(&ctx.db)
            .await?;
        Some(port)
    } else {
        None
    };

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
    let config = &deployment.config;
    let source_archive = config
        .get("source_archive")
        .and_then(|v| v.as_str())
        .filter(|s| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
        .map(String::from);

    // Remote nodes can't see local host paths — uploads and static
    // artifacts live on this machine's data dir.
    if remote_node.is_some() && source_archive.is_some() {
        anyhow::bail!("upload deployments are not supported on remote nodes");
    }

    let mut commands: Vec<String> = if let Some(archive) = &source_archive {
        // Upload deploy: tarball pre-staged by the API under data/uploads,
        // bind-mounted at /src. No git involved.
        let host_root = docker_host_root(&ctx.settings)?;
        binds.push(format!("{}:/src:ro", host_root.join("uploads").display()));
        vec![format!(
            "echo 'Extracting uploaded source...' && \
             tar -xzf /src/{archive} -C /app && \
             echo 'Source extracted'"
        )]
    } else {
        vec![format!(
            "echo 'Cloning {} (Branch: {}, Commit: {})'",
            deployment.repo_full_name,
            deployment.branch,
            &deployment.commit_sha[..7.min(deployment.commit_sha.len())]
        )]
    };

    if source_archive.is_none() {
        match deployment.repo_provider.as_str() {
            "github" | "github_enterprise" => {
                let Some(github) = ctx.github.if_configured() else {
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
                     git fetch -q --depth 1 {base}/{repo}.git {sha} && \
                     git checkout -q FETCH_HEAD && \
                     unset GIT_ASKPASS GIT_TERMINAL_PROMPT RUNWAY_GITHUB_TOKEN && \
                     rm -f /tmp/runway-git-askpass",
                    base = deployment.repo_base_url.trim_end_matches('/'),
                    repo = deployment.repo_full_name,
                    sha = deployment.commit_sha,
                ));
            }
            p @ ("gitea" | "gitlab" | "bitbucket") => {
                // Port of devpush's gitea clone arm — token connection,
                // askpass injection, `<base_url>/<full_name>.git`.
                // Bitbucket clones go to bitbucket.org (api.* is REST).
                let conn_id = match p {
                    "gitea" => project.gitea_connection_id,
                    "gitlab" => project.gitlab_connection_id,
                    _ => project.bitbucket_connection_id,
                }
                .ok_or_else(|| anyhow::anyhow!("project has no {p} connection"))?;
                let conn = runway_core::git_providers::connection(&ctx.db, &ctx.crypto, p, conn_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("{p} connection {conn_id} not found"))?;
                env.push(format!("RUNWAY_GIT_TOKEN={}", conn.token));
                // `repo_base_url` was resolved at project create —
                // bitbucket clones hit bitbucket.org, not api.*.
                let base = deployment.repo_base_url.trim_end_matches('/');
                // Bitbucket app passwords authenticate as the workspace
                // user, not a fixed `x-access-token`.
                let git_user = if p == "bitbucket" {
                    conn.username.clone()
                } else {
                    "x-access-token".into()
                };
                commands.push(format!(
                    "git init -q && \
                     printf '%s\\n' '#!/bin/sh' \
                     'case \"$1\" in *Username*) echo \"{git_user}\";; *) echo \"$RUNWAY_GIT_TOKEN\";; esac' \
                     > /tmp/runway-git-askpass && \
                     chmod 700 /tmp/runway-git-askpass && \
                     export GIT_ASKPASS=/tmp/runway-git-askpass GIT_TERMINAL_PROMPT=0 && \
                     git fetch -q --depth 1 {base}/{repo}.git {sha} && \
                     git checkout -q FETCH_HEAD && \
                     unset GIT_ASKPASS GIT_TERMINAL_PROMPT RUNWAY_GIT_TOKEN && \
                     rm -f /tmp/runway-git-askpass",
                    repo = deployment.repo_full_name,
                    sha = deployment.commit_sha,
                ));
            }
            other => anyhow::bail!("repo provider '{other}' not supported yet"),
        }
    }

    // Static mode: `output_directory` materializes the build to a host
    // dir served by the `static-web` image instead of a runner process.
    let output_dir = config
        .get("output_directory")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_matches('/')
        .to_string();
    let static_mode = !output_dir.is_empty();
    if static_mode && remote_node.is_some() {
        anyhow::bail!("static deploys are not supported on remote nodes (artifact dir is local)");
    }
    let mut spa_fallback = config
        .get("spa_fallback")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let static_local_dir = std::path::PathBuf::from(&ctx.settings.data_dir)
        .join("static")
        .join(deployment_id);
    let static_host_dir = if static_mode {
        tokio::fs::create_dir_all(&static_local_dir).await?;
        let host_dir = docker_host_root(&ctx.settings)?
            .join("static")
            .join(deployment_id)
            .display()
            .to_string();
        binds.push(format!("{host_dir}:/out"));
        Some(host_dir)
    } else {
        None
    };

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
    // `[ -n ... ]` tests the raw value; the subshell needs a no-op when empty.
    let pre_deploy_cmd = if pre_deploy.is_empty() {
        ":"
    } else {
        pre_deploy
    };
    commands.push(format!(
        "if [ -f runway.json ] && command -v jq >/dev/null 2>&1; then \
         OVERRIDE=$(jq -r '.pre_deploy_command // empty' runway.json); \
         if [ -n \"$OVERRIDE\" ]; then \
         echo 'Running pre-deploy command from runway.json...'; \
         ( $OVERRIDE ); \
         elif [ -n '{pre_deploy}' ]; then \
         echo 'Running pre-deploy command...'; \
         ( {pre_deploy_cmd} ); \
         fi; else \
         if [ -n '{pre_deploy}' ]; then \
         echo 'Running pre-deploy command...'; \
         ( {pre_deploy_cmd} ); \
         fi; fi"
    ));
    if static_mode {
        commands.push(format!(
            "OUTDIR=$(if [ -f runway.json ] && command -v jq >/dev/null 2>&1; then \
             jq -r '.output_directory // empty' runway.json; fi); \
             OUTDIR=${{OUTDIR:-{output_dir}}}; \
             echo \"Publishing static output ($OUTDIR -> /out)...\"; \
             test -d \"$OUTDIR\" || {{ printf '\\033[31mError: output directory %s not found\\033[0m\\n' \"$OUTDIR\" 1>&2; exit 1; }}; \
             mkdir -p /out && cp -r \"$OUTDIR\"/. /out/ && \
             (cp runway.json /out/.runway.json 2>/dev/null || true) && \
             echo 'Static output published'"
        ));
    } else {
        commands.push("echo 'Starting application...'".into());
        commands.push(config_command("start_command", cfg_str("start_command")));
    }

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
    let app_port = deployment.serve_port();
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
            runner_image = presets::runner_image_resolved(&ctx.settings.data_dir, slug);
        }
    }
    let Some(runner_image) = runner_image else {
        anyhow::bail!("runner image not found for deployment");
    };

    // Build-output cache for node/bun runners — persists incremental
    // compiler state (.next/cache, .turbo) across deployments.
    if runner_image.contains("node") || runner_image.contains("bun") {
        let base = format!("runway-bcache-{}", &project.id[..12.min(project.id.len())]);
        let root = if root_dir.is_empty() {
            String::new()
        } else {
            format!("/{root_dir}")
        };
        for (suffix, path) in [
            ("next", format!("/app{root}/.next/cache")),
            ("turbo", format!("/app{root}/.turbo")),
        ] {
            let vol = format!("{base}-{suffix}");
            let _ = docker
                .create_volume(CreateVolumeOptions {
                    name: vol.clone(),
                    labels: HashMap::from([
                        ("runway.project_id".into(), project.id.clone()),
                        ("runway.cache".into(), format!("build-{suffix}")),
                    ]),
                    ..Default::default()
                })
                .await;
            binds.push(format!("{vol}:{path}"));
        }
    }

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

    let serve_image = if static_mode {
        let img = presets::runner_image_resolved(&ctx.settings.data_dir, "static-web")
            .expect("static-web runner is registered");
        if docker.inspect_image(&img).await.is_err() {
            log(&format!("Pulling static server image ({img})...")).await;
            pull_image(docker, &img).await?;
        }
        Some(img)
    } else {
        None
    };

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
    // Linked storage: data mounts + network attach (deploy-time wiring).
    // Remote nodes can't reach local storage containers or their host
    // binds — skip and note it in the deploy log.
    let (storage_nets, storage_mounts) = if remote_node.is_some() {
        let has_linked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM storage_project WHERE project_id = $1)",
        )
        .bind(&project.id)
        .fetch_one(&ctx.db)
        .await
        .unwrap_or(false);
        if has_linked {
            log("Note: linked storage is local-only — not attached on remote node").await;
        }
        (vec![], vec![])
    } else {
        crate::storage::linked(
            ctx,
            &project.id,
            if deployment.environment_id.is_empty() {
                None
            } else {
                Some(deployment.environment_id.as_str())
            },
        )
        .await
    };
    binds.extend(storage_mounts.clone());
    if !binds.is_empty() {
        host_config.binds = Some(binds);
    }
    if let Some(port) = remote_port {
        host_config.port_bindings = Some(HashMap::from([(
            format!("{}/tcp", deployment.serve_port()),
            Some(vec![bollard::models::PortBinding {
                host_ip: Some("0.0.0.0".into()),
                host_port: Some(port.to_string()),
            }]),
        )]));
        log(&format!("Publishing node port {port}")).await;
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

    // Serving containers join edge + workspace + linked storage networks
    // (the fix for devpush never attaching storage nets).
    let mut serve_endpoints = HashMap::from([
        (edge_network.clone(), EndpointSettings::default()),
        (workspace_network.clone(), EndpointSettings::default()),
    ]);
    for n in storage_nets {
        serve_endpoints.insert(n, EndpointSettings::default());
    }

    let body = if static_mode {
        // Phase 1: throwaway build container — clones, builds, copies
        // output_directory into the host-mounted /out, then exits.
        let build_name = format!("{container_name}-build");
        let build_body = ContainerConfig {
            image: Some(runner_image.clone()),
            env: Some(env),
            working_dir: Some("/app".into()),
            labels: Some(HashMap::from([
                ("runway.deployment_id".into(), deployment_id.clone()),
                ("runway.project_id".into(), project.id.clone()),
                ("runway.team_id".into(), project.team_id.clone()),
                ("runway.role".into(), "build".into()),
            ])),
            networking_config: Some(NetworkingConfig {
                endpoints_config: HashMap::from([(
                    workspace_network.clone(),
                    EndpointSettings::default(),
                )]),
            }),
            host_config: Some(HostConfig {
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                }),
                ..host_config.clone()
            }),
            cmd: Some(cmd),
            ..Default::default()
        };
        let build_id = match dkr::create_or_replace_container(docker, &build_name, build_body).await
        {
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
            .start_container(&build_id, None::<StartContainerOptions<String>>)
            .await?;
        spawn_log_tailer(ctx, build_id.clone(), deployment_id.clone());
        log("Building static output...").await;
        let exit = wait_exit(docker, &build_id, ctx.settings.deployment_timeout_seconds).await;
        let _ = docker
            .remove_container(
                &build_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        match exit {
            Ok(0) => log("Static build completed").await,
            Ok(code) => {
                deploy::enqueue(
                    &ctx.db,
                    "fail_deployment",
                    json!({
                        "deployment_id": deployment_id,
                        "status": "prepare",
                        "reason": format!("build failed (exit code {code})"),
                    }),
                    0,
                )
                .await?;
                return Ok(());
            }
            Err(e) => {
                deploy::enqueue(
                    &ctx.db,
                    "fail_deployment",
                    json!({
                        "deployment_id": deployment_id,
                        "status": "prepare",
                        "reason": format!("build failed: {e}"),
                    }),
                    0,
                )
                .await?;
                return Ok(());
            }
        }

        // Phase 2: serve the artifact with the static-web image.
        let static_dir = static_host_dir.expect("static_mode implies static_host_dir");

        // runway.json extracted during build → SWS config + overrides.
        let mut serve_binds = vec![format!("{static_dir}:/public:ro")];
        serve_binds.extend(storage_mounts);
        let marker = static_local_dir.join(".runway.json");
        if let Ok(raw) = tokio::fs::read_to_string(&marker).await {
            let _ = tokio::fs::remove_file(&marker).await; // never served
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(spa) = val.get("spa_fallback").and_then(|v| v.as_bool()) {
                    spa_fallback = spa;
                }
                if let Some(toml) = sws_config(&val) {
                    let toml_path = std::path::PathBuf::from(&ctx.settings.data_dir)
                        .join("static")
                        .join(format!("{deployment_id}.toml"));
                    if tokio::fs::write(&toml_path, toml).await.is_ok() {
                        let host_toml = static_dir
                            .trim_end_matches(&format!("/{deployment_id}"))
                            .to_string()
                            + &format!("/{deployment_id}.toml");
                        serve_binds.push(format!("{host_toml}:/config.toml:ro"));
                    }
                }
            }
        }

        let mut serve_env = vec!["SERVER_ROOT=/public".to_string()];
        if serve_binds.len() > 1 {
            serve_env.push("SERVER_CONFIG_FILE=/config.toml".to_string());
        }
        if spa_fallback {
            serve_env.push("SERVER_PAGE_FALLBACK=/public/index.html".to_string());
        }
        ContainerConfig {
            image: serve_image,
            env: Some(serve_env),
            labels: Some(labels),
            networking_config: Some(NetworkingConfig {
                endpoints_config: serve_endpoints.clone(),
            }),
            host_config: Some(HostConfig {
                binds: Some(serve_binds),
                ..host_config
            }),
            ..Default::default()
        }
    } else {
        ContainerConfig {
            image: Some(runner_image),
            env: Some(env),
            working_dir: Some("/app".into()),
            labels: Some(labels),
            networking_config: Some(NetworkingConfig {
                endpoints_config: serve_endpoints,
            }),
            host_config: Some(host_config),
            cmd: Some(cmd),
            ..Default::default()
        }
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

    // "started" fires once the container is up, per devpush.
    if let Ok(Some(dep_now)) = deploy::get(&ctx.db, deployment_id).await {
        runway_core::webhook::send_deployment_webhooks(
            &ctx.db,
            &ctx.crypto,
            &ctx.settings,
            project,
            &dep_now,
            "started",
        )
        .await;
    }

    spawn_log_tailer(ctx, container_id.clone(), deployment_id.clone());
    Ok(())
}

/// `if [ -f runway.json ] && jq ...` wrapper for a config-command step.
fn config_command(key: &str, command: &str) -> String {
    // `( )` is a parse error in dash — substitute a no-op for empty commands.
    let command = if command.is_empty() { ":" } else { command };
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

/// Translate runway.json `redirects`/`rewrites`/`headers` (vercel.json
/// shape) into a static-web-server `advanced` config. None when the
/// file carries none of them.
fn sws_config(val: &serde_json::Value) -> Option<String> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::new();

    if let Some(redirects) = val.get("redirects").and_then(|v| v.as_array()) {
        for r in redirects {
            let (Some(src), Some(dst)) = (r["source"].as_str(), r["destination"].as_str()) else {
                continue;
            };
            let kind = r["status_code"]
                .as_i64()
                .or_else(|| r["status"].as_i64())
                .unwrap_or(if r["permanent"].as_bool() == Some(true) {
                    308
                } else {
                    307
                });
            out += &format!(
                "[[advanced.redirects]]\nsource = \"{}\"\ndestination = \"{}\"\nkind = {kind}\n\n",
                esc(src),
                esc(dst)
            );
        }
    }
    if let Some(rewrites) = val.get("rewrites").and_then(|v| v.as_array()) {
        for r in rewrites {
            let (Some(src), Some(dst)) = (r["source"].as_str(), r["destination"].as_str()) else {
                continue;
            };
            out += &format!(
                "[[advanced.rewrites]]\nsource = \"{}\"\ndestination = \"{}\"\n\n",
                esc(src),
                esc(dst)
            );
        }
    }
    if let Some(headers) = val.get("headers").and_then(|v| v.as_array()) {
        for h in headers {
            let Some(src) = h["source"].as_str() else {
                continue;
            };
            let entries = h["headers"].as_array();
            let mut block = format!("[[advanced.headers]]\nsource = \"{}\"\n", esc(src));
            if let Some(entries) = entries {
                for e in entries {
                    let (Some(k), Some(v)) = (e["key"].as_str(), e["value"].as_str()) else {
                        continue;
                    };
                    block += &format!(
                        "[[advanced.headers.headers]]\nkey = \"{}\"\nvalue = \"{}\"\n",
                        esc(k),
                        esc(v)
                    );
                }
            }
            out += &block;
            out += "\n\n";
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(format!("[advanced]\n{out}"))
    }
}

/// Wait for a container to exit; returns its exit code.
pub async fn wait_exit(
    docker: &bollard::Docker,
    id: &str,
    timeout_secs: u64,
) -> anyhow::Result<i64> {
    use std::time::Duration;
    let mut stream = docker.wait_container::<String>(id, None);
    let code = tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), async {
        match stream.next().await {
            Some(res) => anyhow::Ok(res?.status_code),
            None => anyhow::bail!("container wait stream ended before exit"),
        }
    })
    .await??;
    Ok(code)
}

pub async fn pull_image(docker: &bollard::Docker, image: &str) -> anyhow::Result<()> {
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
    post_commit_status(ctx, &deployment, &project, "success", "Deployment ready").await;
    let dep_now = deploy::get(&ctx.db, deployment_id)
        .await?
        .unwrap_or_else(|| deployment.clone());
    runway_core::webhook::send_deployment_webhooks(
        &ctx.db,
        &ctx.crypto,
        &ctx.settings,
        &project,
        &dep_now,
        "succeeded",
    )
    .await;
    runway_core::audit::notify_team(
        &ctx.db,
        &project.team_id,
        "deployment.succeeded",
        &format!("Deployment ready: {}", project.name),
        runway_core::audit::Notify {
            link: Some(&format!(
                "/projects/{}/deployments/{deployment_id}",
                project.id
            )),
            project_id: Some(&project.id),
            ..Default::default()
        },
    )
    .await;

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
    if let Ok(Some(project)) = deploy::get_project(&ctx.db, &deployment.project_id).await {
        post_commit_status(
            ctx,
            &deployment,
            &project,
            "failure",
            reason.unwrap_or("Deployment failed"),
        )
        .await;
        if let Ok(Some(dep_now)) = deploy::get(&ctx.db, deployment_id).await {
            runway_core::webhook::send_deployment_webhooks(
                &ctx.db,
                &ctx.crypto,
                &ctx.settings,
                &project,
                &dep_now,
                "failed",
            )
            .await;
        }
        runway_core::audit::notify_team(
            &ctx.db,
            &project.team_id,
            "deployment.failed",
            &format!("Deployment failed: {}", project.name),
            runway_core::audit::Notify {
                body: Some(reason.unwrap_or("Deployment failed")),
                link: Some(&format!(
                    "/projects/{}/deployments/{deployment_id}",
                    project.id
                )),
                project_id: Some(&project.id),
                ..Default::default()
            },
        )
        .await;
    }
    Ok(())
}

/// GitHub commit status for the deployment — powers PR preview checks.
/// Best-effort: no token configured, no status posted.
async fn post_commit_status(
    ctx: &Ctx,
    deployment: &Deployment,
    project: &Project,
    state: &str,
    description: &str,
) {
    let (Some(gh), Some(installation_id)) =
        (ctx.github.if_configured(), project.github_installation_id)
    else {
        return;
    };
    let Ok(token) = gh
        .installation_token(&ctx.db, &ctx.crypto, installation_id)
        .await
    else {
        return;
    };
    let project_slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let url = deployment.url(&project_slug, &ctx.settings);
    if let Err(e) = gh
        .commit_status(
            &token,
            &deployment.repo_full_name,
            &deployment.commit_sha,
            state,
            Some(&url),
            description,
        )
        .await
    {
        tracing::warn!(deployment_id = deployment.id, error = %e, "commit status post failed");
    }
}

/// Resolve the daemon a deployment's container lives on.
async fn dep_docker(ctx: &Ctx, remote_node_id: Option<&str>) -> Option<bollard::Docker> {
    match remote_node_id {
        Some(nid) => dkr::node_client(&ctx.db, nid).await,
        None => Some(ctx.docker.clone()),
    }
}

/// `delete_container` — remove a stopped deployment container + edge network.
pub async fn delete_container(ctx: &Ctx, deployment_id: &str) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        return Ok(());
    };
    let Some(container_id) = &deployment.container_id else {
        return Ok(());
    };

    let Some(docker) = dep_docker(ctx, deployment.remote_node_id.as_deref()).await else {
        // Node unreachable — leave the row for a later retry.
        tracing::warn!(deployment_id, "remote node unreachable, delete deferred");
        return Ok(());
    };

    match docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
    {
        Ok(info) => {
            let edge = dkr::container_label(&info, "runway.edge_network")
                .unwrap_or_else(|| edge_network_name(deployment_id));
            let _ = docker
                .stop_container(container_id, None::<StopContainerOptions>)
                .await;
            docker
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
            if deployment.remote_node_id.is_none() {
                let traefik_id = dkr::service_container_id(&ctx.docker, "traefik").await;
                dkr::disconnect_from_network(&ctx.docker, traefik_id.as_deref(), Some(&edge))
                    .await?;
            }
            let _ = dkr::remove_network_if_empty(&docker, &edge).await;
        }
        Err(e) if dkr::is_not_found(&e) => {
            sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                .bind(deployment_id)
                .execute(&ctx.db)
                .await?;
            if deployment.remote_node_id.is_some() {
                // Container already gone on the node — drop its empty
                // edge network there.
                let _ =
                    dkr::remove_network_if_empty(&docker, &edge_network_name(deployment_id)).await;
            } else {
                deploy::cleanup_edge_network(&ctx.docker, deployment_id).await?;
            }
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
        let Some(docker) = dep_docker(ctx, dep.remote_node_id.as_deref()).await else {
            continue;
        };
        match docker
            .inspect_container(&container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => {
                let _ = docker
                    .stop_container(&container_id, None::<StopContainerOptions>)
                    .await;
                let _ = docker
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
                if dep.remote_node_id.is_none() {
                    let traefik_id = dkr::service_container_id(&ctx.docker, "traefik").await;
                    dkr::disconnect_from_network(&ctx.docker, traefik_id.as_deref(), Some(&edge))
                        .await?;
                }
                let _ = dkr::remove_network_if_empty(&docker, &edge).await;
                drop_artifacts(ctx, dep).await;
            }
            Err(e) if dkr::is_not_found(&e) => {
                sqlx::query("UPDATE deployment SET container_status = 'removed' WHERE id = $1")
                    .bind(&dep.id)
                    .execute(&ctx.db)
                    .await?;
                drop_artifacts(ctx, dep).await;
            }
            Err(e) => {
                tracing::warn!(deployment_id = dep.id, error = %e, "cleanup failed for container")
            }
        }
    }
    Ok(())
}

/// Host-visible path for bind mounts: `host_data_dir` when runway itself runs
/// in a container (compose), else `data_dir` resolved to an absolute path —
/// docker rejects relative bind sources.
pub fn docker_host_root(settings: &runway_core::Settings) -> anyhow::Result<std::path::PathBuf> {
    let root = settings
        .host_data_dir
        .clone()
        .unwrap_or_else(|| settings.data_dir.clone());
    let root = std::path::PathBuf::from(root);
    Ok(if root.is_absolute() {
        root
    } else {
        std::env::current_dir()?.join(root)
    })
}

/// Remove host-side deployment artifacts: static output dir + upload tarball.
/// The upload tarball is retained until this point because `redeploy` of an
/// upload deployment re-extracts it.
async fn drop_artifacts(ctx: &Ctx, dep: &Deployment) {
    let data = std::path::Path::new(&ctx.settings.data_dir);
    let _ = tokio::fs::remove_dir_all(data.join("static").join(&dep.id)).await;
    if let Some(archive) = dep.config.get("source_archive").and_then(|v| v.as_str()) {
        let _ = tokio::fs::remove_file(data.join("uploads").join(archive)).await;
    }
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
