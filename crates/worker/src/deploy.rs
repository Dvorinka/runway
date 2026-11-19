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
    ctx.logs.step(deployment_id, "prepare").await;

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
    } else if let Some(selected) = select_node_by_labels(&ctx.db, project).await? {
        let client = runway_core::docker::docker_client(&selected.docker_url)?;
        tracing::info!(node = %selected.id, name = %selected.name,
            "node_selector matched remote node");
        remote_node = Some(selected);
        Some(client)
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
             echo 'Source extracted' && \
             touch /app/.runway-src-ok"
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
                // Token lives only inside the askpass file, which is
                // deleted before user build commands run — env vars
                // would leak via /proc/1/environ (fork-PR builds run
                // untrusted code).
                commands.push(format!(
                    "git init -q && \
                     echo {askpass} | base64 -d > /tmp/runway-git-askpass && \
                     chmod 700 /tmp/runway-git-askpass && \
                     export GIT_ASKPASS=/tmp/runway-git-askpass GIT_TERMINAL_PROMPT=0 && \
                     git fetch -q --depth 1 {base}/{repo}.git {sha} && \
                     git checkout -q FETCH_HEAD && \
                     unset GIT_ASKPASS GIT_TERMINAL_PROMPT && \
                     rm -f /tmp/runway-git-askpass && \
                     touch /app/.runway-src-ok",
                    askpass = b64_askpass("x-access-token", &token),
                    base = deployment.repo_base_url.trim_end_matches('/'),
                    repo = deployment.repo_full_name,
                    sha = deployment.commit_sha,
                ));
            }
            p @ ("gitea" | "forgejo" | "gitlab" | "bitbucket") => {
                // Port of devpush's gitea clone arm — token connection,
                // askpass injection, `<base_url>/<full_name>.git`.
                // Bitbucket clones go to bitbucket.org (api.* is REST).
                let conn_id = match p {
                    "gitea" | "forgejo" => project.gitea_connection_id,
                    "gitlab" => project.gitlab_connection_id,
                    _ => project.bitbucket_connection_id,
                }
                .ok_or_else(|| anyhow::anyhow!("project has no {p} connection"))?;
                let conn = runway_core::git_providers::connection(&ctx.db, &ctx.crypto, p, conn_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("{p} connection {conn_id} not found"))?;
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
                     echo {askpass} | base64 -d > /tmp/runway-git-askpass && \
                     chmod 700 /tmp/runway-git-askpass && \
                     export GIT_ASKPASS=/tmp/runway-git-askpass GIT_TERMINAL_PROMPT=0 && \
                     git fetch -q --depth 1 {base}/{repo}.git {sha} && \
                     git checkout -q FETCH_HEAD && \
                     unset GIT_ASKPASS GIT_TERMINAL_PROMPT && \
                     rm -f /tmp/runway-git-askpass && \
                     touch /app/.runway-src-ok",
                    askpass = b64_askpass(&git_user, &conn.token),
                    base = base,
                    repo = deployment.repo_full_name,
                    sha = deployment.commit_sha,
                ));
            }
            other => anyhow::bail!("repo provider '{other}' not supported yet"),
        }
    }

    // Source acquisition only — reused by dockerfile builds, which stage
    // the repo on the host instead of inside the serve container.
    let source_cmds = commands.clone();

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
    // Optional KERN executor: the identical build script runs on a remote
    // KERN runner; its `runway-out` artifact lands in static_local_dir and
    // phase 2 serves it unchanged. Static mode only.
    let kern_build = config.get("build_executor").and_then(|v| v.as_str()) == Some("kern");
    if kern_build && !static_mode {
        anyhow::bail!("build_executor=kern requires output_directory (static mode)");
    }
    // Dockerfile mode: the repo builds into an image via `docker build`;
    // the image's own CMD/ENTRYPOINT serves (or `entrypoint` overrides).
    let dockerfile_path = config
        .get("dockerfile_path")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().trim_start_matches("./").to_string())
        .filter(|s| !s.is_empty());
    let docker_build = dockerfile_path.is_some();
    if docker_build && static_mode {
        anyhow::bail!("dockerfile_path cannot be combined with output_directory");
    }
    if let Some(df) = &dockerfile_path {
        if !safe_rel_path(df) {
            anyhow::bail!("dockerfile_path must be a relative path inside the repo: {df}");
        }
    }

    let mut spa_fallback = config
        .get("spa_fallback")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let static_local_dir = std::path::PathBuf::from(&ctx.settings.data_dir)
        .join("static")
        .join(deployment_id);
    // Remote builds keep /out in-container; it is pulled back over the
    // docker API after the build exits, then shipped to a node volume.
    let static_host_dir = if static_mode {
        tokio::fs::create_dir_all(&static_local_dir).await?;
        if remote_node.is_none() {
            let host_dir = docker_host_root(&ctx.settings)?
                .join("static")
                .join(deployment_id)
                .display()
                .to_string();
            binds.push(format!("{host_dir}:/out"));
            Some(host_dir)
        } else {
            None
        }
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
    if !root_dir.is_empty() && !docker_build {
        commands.push(format!("echo 'Changing root directory to {root_dir}'"));
        commands.push(format!(
            "test -d {root_dir} || {{ printf '\\033[31mError: root directory %s not found\\033[0m\\n' {root_dir} 1>&2; exit 1; }}"
        ));
        commands.push(format!("cd {root_dir}"));
    }

    if !docker_build {
        // Build / pre-deploy / start — runway.json overrides are resolved
        // in-container. Runner images are upstream and may lack jq; fall
        // back to node, then python3, so the overrides work on every
        // runtime that ships a JSON-capable interpreter.
        commands.push(
            "readcfg() { if [ -f runway.json ]; then \
             if command -v jq >/dev/null 2>&1; then jq -r \".$1 // empty\" runway.json; \
             elif command -v node >/dev/null 2>&1; then \
             node -e \"try{var v=require('./runway.json')[process.argv[1]];process.stdout.write(v==null?'':String(v))}catch(e){}\" \"$1\"; \
             elif command -v python3 >/dev/null 2>&1; then \
             python3 -c \"import json,sys;print(json.load(open('runway.json')).get(sys.argv[1]) or '',end='')\" \"$1\" 2>/dev/null; \
             fi; fi; }"
                .to_string(),
        );
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
        // Braced group: entries join with `&&`, so each must stay a single
        // compound command or a `;` would break the chain and leak a stale
        // OVERRIDE into the next step. eval re-parses `&&` inside overrides.
        commands.push(format!(
            "{{ OVERRIDE=$(readcfg pre_deploy_command); \
         if [ -n \"$OVERRIDE\" ]; then \
         echo 'Running pre-deploy command from runway.json...'; \
         ( eval \"$OVERRIDE\" ); \
         elif [ -n '{pre_deploy}' ]; then \
         echo 'Running pre-deploy command...'; \
         ( {pre_deploy_cmd} ); \
         fi; }}"
        ));
        if static_mode {
            commands.push(format!(
            "test -f /app/.runway-src-ok || {{ printf '\\033[31mError: source acquisition failed — refusing to publish a stale workspace\\033[0m\\n' 1>&2; exit 1; }}; \
             OUTDIR=$(readcfg output_directory); \
             OUTDIR=${{OUTDIR:-{output_dir}}}; \
             echo \"Publishing static output ($OUTDIR -> /out)...\"; \
             test -d \"$OUTDIR\" || {{ printf '\\033[31mError: output directory %s not found\\033[0m\\n' \"$OUTDIR\" 1>&2; exit 1; }}; \
             mkdir -p /out/.w && \
             tar -cf - --exclude='./.git' --exclude='./.runway-src-ok' --exclude='./.runway-out' -C \"$OUTDIR\" . | tar -xmf - -C /out/.w && cp -r /out/.w/. /out/ && rm -rf /out/.w && \
             test -n \"$(find /out -type f -print -quit)\" || {{ printf '\\033[31mError: build produced no files\\033[0m\\n' 1>&2; exit 1; }} && \
             (cp runway.json /out/.runway.json 2>/dev/null || true) && \
             echo 'Static output published'"
        ));
            // Analytics + speed-insights snippets: injected post-publish into
            // every .html under /out. Passed via env so arbitrary HTML never
            // touches shell interpolation.
            let mut snippets: Vec<&str> = Vec::new();
            if let Some(s) = config
                .get("analytics_meta")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                snippets.push(s);
            }
            if let Some(s) = config
                .get("analytics_snippet")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                snippets.push(s);
            }
            let speed_snippet;
            let beacon_on = ["speed_insights", "web_analytics"].iter().any(|k| {
                config
                    .get(k)
                    .and_then(|v| v.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            });
            if beacon_on {
                speed_snippet = runway_core::rum::beacon_snippet();
                snippets.push(speed_snippet);
            }
            if !snippets.is_empty() {
                env.push(format!("RUNWAY_SNIPPETS={}", snippets.join("\n<!-- -->\n")));
                commands.push(
                "if command -v node >/dev/null 2>&1; then \
                 echo 'Injecting head snippets...'; \
                 node -e 'const fs=require(\"fs\"),p=require(\"path\"),ss=process.env.RUNWAY_SNIPPETS.split(\"\\n<!-- -->\\n\"); \
                 function w(d){for(const f of fs.readdirSync(d)){const q=p.join(d,f); \
                 if(fs.statSync(q).isDirectory()){w(q)}else if(f.endsWith(\".html\")){let h=fs.readFileSync(q,\"utf8\"); \
                 for(const s of ss){if(!h.includes(s)){ \
                 var t=s.trimStart().startsWith(\"<meta\")&&h.includes(\"</head>\")?\"</head>\":\"</body>\"; \
                 h=h.includes(t)?h.replace(t,s+t):h+s;}} \
                 fs.writeFileSync(q,h)}}} \
                 w(\"/out\")'; \
                 else echo 'Snippet injection skipped (node unavailable)'; fi"
                    .to_string(),
            );
            }
        } else {
            commands.push("echo 'Starting application...'".into());
            commands.push(config_command("start_command", cfg_str("start_command")));
        }
    }
    if static_mode && remote_node.is_some() {
        // tmpfs /out unmounts when the build container exits — persist
        // into the writable layer so the archive API can read it back.
        commands.push("cp -a /out/. /app/.runway-out/".into());
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
    // devpush parity: uncapped unless the operator sets defaults; custom
    // per-project values are honored only when a default+max pair exists.
    let mut cpus = ctx.settings.default_cpus;
    let mut memory_mb = ctx.settings.default_memory_mb;
    if let (Some(_), Some(max)) = (ctx.settings.default_cpus, ctx.settings.max_cpus) {
        if let Some(o) = config.get("cpus").and_then(|v| v.as_f64()) {
            if o > 0.0 {
                cpus = Some(f64::min(o, max));
            }
        }
    }
    if let (Some(_), Some(max)) = (ctx.settings.default_memory_mb, ctx.settings.max_memory_mb) {
        if let Some(o) = config.get("memory").and_then(|v| v.as_i64()) {
            if o > 0 {
                memory_mb = Some(i64::min(o, max));
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
                .and_then(|v| v.as_str());
            runner_image =
                slug.and_then(|s| presets::runner_image_resolved(&ctx.settings.data_dir, s));
        }
    }
    if docker_build && runner_image.is_none() {
        // Dockerfile deploys don't need a preset runner — only a fetch
        // stage with git+tar to stage the repo on the host.
        // jarvis: floating tag; pin once runner-overrides carry a fetch image.
        runner_image = Some("alpine/git:latest".into());
    }
    let Some(mut runner_image) = runner_image else {
        anyhow::bail!("runner image not found for deployment");
    };

    // Build-output cache for node/bun runners — persists incremental
    // compiler state (.next/cache, .turbo) across deployments. Skipped for
    // dockerfile builds — those mounts would shadow image-baked paths.
    if !docker_build && (runner_image.contains("node") || runner_image.contains("bun")) {
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

    if docker_build {
        runner_image = dockerfile_build(
            ctx,
            docker,
            deployment,
            &log,
            ImageBuildSpec {
                dockerfile: dockerfile_path.as_deref().unwrap_or_default(),
                root_dir: &root_dir,
                fetch_image: &runner_image,
                source_cmds: &source_cmds,
                env: &env,
            },
        )
        .await?;
        // Usage metering: built image bytes, for the usage endpoint.
        if let Ok(info) = docker.inspect_image(&runner_image).await {
            if let Some(bytes) = info.size {
                let _ =
                    sqlx::query("UPDATE deployment SET config = config || $1::jsonb WHERE id = $2")
                        .bind(json!({ "image_bytes": bytes }))
                        .bind(&deployment.id)
                        .execute(&ctx.db)
                        .await;
            }
        }
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
    if let Some(c) = cpus {
        host_config.nano_cpus = Some((c * 1e9) as i64);
    }
    if let Some(m) = memory_mb {
        host_config.memory = Some(m * 1024 * 1024);
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
    if docker_build {
        // Serve container = the built image; source/cache binds would
        // shadow baked content. Linked storage mounts still apply.
        binds = storage_mounts.clone();
    } else {
        binds.extend(storage_mounts.clone());
    }
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
        if kern_build {
            // Phase 1 (KERN): remote runner executes the same script; the
            // uploaded artifact fills static_local_dir. No local container.
            kern_static_build(
                ctx,
                deployment,
                &commands,
                &env,
                &runner_image,
                source_archive.as_deref(),
                &static_local_dir,
                &log,
            )
            .await?;
        } else {
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
                    // Remote builds write /out in-container; a tmpfs
                    // keeps the non-root runner image able to create it.
                    tmpfs: remote_node
                        .is_some()
                        .then(|| HashMap::from([("/out".to_string(), "rw".to_string())])),
                    ..host_config.clone()
                }),
                cmd: Some(cmd),
                ..Default::default()
            };
            let build_id =
                match dkr::create_or_replace_container(docker, &build_name, build_body).await {
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
            if remote_node.is_some() && matches!(exit, Ok(0)) {
                download_static_out(docker, &build_id, &static_local_dir).await?;
                log("Build artifact returned from node").await;
            }
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
                Ok(0) => {
                    log("Static build completed").await;
                    // Usage metering: static artifact bytes, recorded on
                    // the deployment for the project usage endpoint.
                    let dir = static_local_dir.clone();
                    let bytes = tokio::task::spawn_blocking(move || {
                        walkdir_size(std::path::Path::new(&dir))
                    })
                    .await
                    .unwrap_or(0);
                    if bytes > 0 {
                        let _ = sqlx::query(
                            "UPDATE deployment SET config = config || $1::jsonb WHERE id = $2",
                        )
                        .bind(json!({ "artifact_bytes": bytes }))
                        .bind(&deployment.id)
                        .execute(&ctx.db)
                        .await;
                    }
                }
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
        }

        // Phase 2: serve the artifact with the static-web image.
        // runway.json extracted during build → SWS config + overrides.
        let mut serve_toml: Option<String> = None;
        let marker = static_local_dir.join(".runway.json");
        if let Ok(raw) = tokio::fs::read_to_string(&marker).await {
            let _ = tokio::fs::remove_file(&marker).await; // never served
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(spa) = val.get("spa_fallback").and_then(|v| v.as_bool()) {
                    spa_fallback = spa;
                }
                serve_toml = sws_config(&val, spa_fallback);
            }
        }

        let (serve_env, serve_binds, serve_mounts) = if remote_node.is_none() {
            let static_dir = static_host_dir.expect("local static_mode implies static_host_dir");
            let mut binds = vec![format!("{static_dir}:/public:ro")];
            binds.extend(storage_mounts);
            if let Some(toml) = &serve_toml {
                let toml_path = std::path::PathBuf::from(&ctx.settings.data_dir)
                    .join("static")
                    .join(format!("{deployment_id}.toml"));
                if tokio::fs::write(&toml_path, toml).await.is_ok() {
                    let host_toml = static_dir
                        .trim_end_matches(&format!("/{deployment_id}"))
                        .to_string()
                        + &format!("/{deployment_id}.toml");
                    binds.push(format!("{host_toml}:/config.toml:ro"));
                }
            }
            let mut env = vec!["SERVER_ROOT=/public".to_string()];
            if binds.len() > 1 {
                env.push("SERVER_CONFIG_FILE=/config.toml".to_string());
            }
            if spa_fallback {
                env.push("SERVER_PAGE_FALLBACK=/public/index.html".to_string());
            }
            (env, binds, Vec::new())
        } else {
            // Remote node: ship the artifact into a node-local volume —
            // bind mounts can't reach our filesystem. Layout:
            //   <vol>/public/<files>  <vol>/_cfg/config.toml
            let vol = format!("runway-static-{deployment_id}");
            log("Shipping artifact to node volume...").await;
            ship_static_volume(
                docker,
                &vol,
                &static_local_dir,
                serve_toml.as_deref(),
                &runner_image,
            )
            .await?;
            let mut env = vec!["SERVER_ROOT=/srv/public".to_string()];
            if serve_toml.is_some() {
                env.push("SERVER_CONFIG_FILE=/srv/_cfg/config.toml".to_string());
            }
            if spa_fallback {
                env.push("SERVER_PAGE_FALLBACK=/srv/public/index.html".to_string());
            }
            (
                env,
                Vec::new(),
                vec![bollard::models::Mount {
                    target: Some("/srv".into()),
                    source: Some(vol),
                    typ: Some(bollard::models::MountTypeEnum::VOLUME),
                    read_only: Some(true),
                    ..Default::default()
                }],
            )
        };

        ContainerConfig {
            image: serve_image,
            env: Some(serve_env),
            labels: Some(labels),
            networking_config: Some(NetworkingConfig {
                endpoints_config: serve_endpoints.clone(),
            }),
            host_config: Some(HostConfig {
                binds: if serve_binds.is_empty() {
                    None
                } else {
                    Some(serve_binds)
                },
                mounts: if serve_mounts.is_empty() {
                    None
                } else {
                    Some(serve_mounts)
                },
                ..host_config
            }),
            ..Default::default()
        }
    } else {
        // Dockerfile builds: image's own CMD/ENTRYPOINT (or the
        // entrypoint override) runs; git tokens stay in the fetch
        // container, not the long-lived serve env.
        let serve_env = if docker_build {
            env.iter()
                .filter(|e| {
                    !e.starts_with("RUNWAY_GITHUB_TOKEN=") && !e.starts_with("RUNWAY_GIT_TOKEN=")
                })
                .cloned()
                .collect()
        } else {
            env
        };
        ContainerConfig {
            image: Some(runner_image),
            env: Some(serve_env),
            working_dir: if docker_build {
                None
            } else {
                Some("/app".into())
            },
            labels: Some(labels),
            networking_config: Some(NetworkingConfig {
                endpoints_config: serve_endpoints,
            }),
            host_config: Some(host_config),
            cmd: if docker_build {
                entrypoint.map(|ep| vec!["sh".to_string(), "-c".to_string(), ep.to_string()])
            } else {
                Some(cmd)
            },
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
    ctx.logs.step(deployment_id, "deploy").await;

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

/// Askpass script with credentials baked in, base64-encoded. The token
/// exists only inside the file, which the fetch command deletes before
/// user build commands run — env vars would leak via /proc/1/environ.
fn b64_askpass(user: &str, token: &str) -> String {
    use base64::Engine;
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in *Username*) echo \"{user}\";; *) echo \"{token}\";; esac\n"
    );
    base64::engine::general_purpose::STANDARD.encode(script)
}

/// runway.json override wrapper for a config-command step (see `readcfg`).
fn config_command(key: &str, command: &str) -> String {
    // `( )` is a parse error in dash — substitute a no-op for empty commands.
    let command = if command.is_empty() { ":" } else { command };
    format!(
        "{{ OVERRIDE=$(readcfg {key}); \
         if [ -n \"$OVERRIDE\" ]; then \
         echo 'Using {key} from runway.json'; \
         ( eval \"$OVERRIDE\" ); \
         else \
         ( {command} ); \
         fi; }}"
    )
}

/// Translate runway.json `redirects`/`rewrites`/`headers` (vercel.json
/// shape) into a static-web-server `advanced` config. None when the
/// file carries none of them.
fn sws_config(val: &serde_json::Value, spa_fallback: bool) -> Option<String> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut rules = String::new();

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
            rules += &format!(
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
            rules += &format!(
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
            rules += &block;
            rules += "\n\n";
        }
    }

    if rules.is_empty() && !spa_fallback {
        return None;
    }
    // A config file replaces SERVER_* envs wholesale — root (and the SPA
    // fallback when requested) must be restated under [general].
    let mut out = String::from("[general]\nroot = \"/public\"\n");
    if spa_fallback {
        out += "page-fallback = \"/public/index.html\"\n";
    }
    if !rules.is_empty() {
        out += &format!("\n[advanced]\n{rules}");
    }
    Some(out)
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
    ctx.logs.step(deployment_id, "finalize").await;

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
    // Tunnel mode: exact CNAMEs for every served hostname so Cloudflare
    // issues edge certs (free plans don't cover second-level wildcards).
    // Best-effort: the deployment itself is healthy either way.
    if ctx.settings.cf_api_token.is_some() {
        if let Err(e) = ensure_deploy_dns(ctx, &deployment, &project).await {
            tracing::warn!(deployment_id, error = %e, "deploy DNS ensure failed");
            ctx.logs
                .info(deployment_id, &format!("DNS setup needs attention: {e}"))
                .await;
        }
    }
    ctx.logs.info(deployment_id, "Deployment succeeded").await;
    // Boundary marker: everything after `live` is post-deploy output
    // (container stdout + lifecycle events) — the dashboard splits the
    // log pane into build vs runtime on this marker.
    ctx.logs.step(deployment_id, "live").await;
    post_commit_status(ctx, &deployment, &project, "success", "Deployment ready").await;
    post_preview_comment(ctx, &deployment, &project, "ready").await;
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
            link: Some(&format!("/deployments/{deployment_id}")),
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

/// Exact proxied CNAMEs for a deployment's served hostnames, pointed at
/// the instance tunnel. See `finalize` for why (edge certificates).
async fn ensure_deploy_dns(
    ctx: &Ctx,
    deployment: &Deployment,
    project: &Project,
) -> anyhow::Result<()> {
    let Some(token) = ctx.settings.cf_api_token.clone() else {
        return Ok(());
    };
    let state = runway_core::tunnel::read_tunnel_state(&ctx.settings.data_dir)
        .await
        .ok_or_else(|| anyhow::anyhow!("no instance tunnel state"))?;
    let cf = runway_core::cloudflare::CloudflareClient::new(token);
    let target = format!("{}.cfargotunnel.com", state.tunnel_id);
    let domains = deploy::alias_domains(deployment, project, &ctx.settings);
    for (key, hostname) in &domains {
        if !key.ends_with("_domain") {
            continue;
        }
        cf.create_or_update_dns_record(hostname, &target, true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no cloudflare zone covers {hostname}"))?;
    }
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
        post_preview_comment(ctx, &deployment, &project, "failed").await;
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
                link: Some(&format!("/deployments/{deployment_id}")),
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
    // `runway/<slug>` so monorepos with several projects on one repo get
    // independent required-status checks.
    let context = format!("runway/{project_slug}");
    // Prefer the Checks API — the PR gets a native Re-run button
    // (check_run.rerequested → webhook → redeploy). Older apps registered
    // without checks:write fall back to commit statuses.
    let conclusion = match state {
        "success" => Some("success"),
        _ => Some("failure"),
    };
    let checked = gh
        .create_check_run(
            &token,
            &deployment.repo_full_name,
            &deployment.commit_sha,
            &context,
            "completed",
            conclusion,
            Some(&url),
            &deployment.id,
        )
        .await;
    if checked.is_err() {
        if let Err(e) = gh
            .commit_status(
                &token,
                &deployment.repo_full_name,
                &deployment.commit_sha,
                state,
                Some(&url),
                description,
                &context,
            )
            .await
        {
            tracing::warn!(deployment_id = deployment.id, error = %e, "commit status post failed");
        }
    }

    // PR preview deployments carry `pr_number` — update the upserted
    // comment with the final state and live URL.
    let Some(pr_number) = deployment.config.get("pr_number").and_then(|v| v.as_i64()) else {
        return;
    };
    let preview_url = format!(
        "{}://{}-branch-{}.{}",
        ctx.settings.url_scheme,
        project_slug,
        runway_core::slugify::branch_slug(&deployment.branch),
        ctx.settings.deploy_domain
    );
    let status_line = if state == "success" {
        format!(
            "**Runway preview** — [{preview_url}]({preview_url})\n\nDeployment `{:.7}` is ready.",
            deployment.commit_sha
        )
    } else {
        format!(
            "**Runway preview** — [{preview_url}]({preview_url})\n\nDeployment `{:.7}` {state}.",
            deployment.commit_sha
        )
    };
    let marker = format!("<!-- runway-preview:{} -->", project.id);
    if let Err(e) = gh
        .upsert_issue_comment(
            &token,
            &deployment.repo_full_name,
            pr_number,
            &marker,
            &status_line,
        )
        .await
    {
        tracing::warn!(deployment_id = deployment.id, error = %e, "preview comment update failed");
    }
}

/// Refresh the PR preview comment with the final state — same upsert
/// the API webhook writes at enqueue time. Only runs for PR previews
/// (`config.pr_number`) when `preview_comment` is not "off"/false and
/// verbosity is "full" (minimal comments carry no status).
async fn post_preview_comment(
    ctx: &Ctx,
    deployment: &Deployment,
    project: &Project,
    outcome: &str,
) {
    let Some(pr_number) = deployment.config.get("pr_number").and_then(|v| v.as_i64()) else {
        return;
    };
    let rules = deployment
        .config
        .get("deployment_rules")
        .cloned()
        .unwrap_or(json!({}));
    match rules.get("preview_comment") {
        Some(serde_json::Value::Bool(false)) => return,
        // "minimal" and "off" comments carry no status text.
        Some(serde_json::Value::String(s)) if s.as_str() != "full" => return,
        _ => {}
    }
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
    let slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let host = format!(
        "{slug}-branch-{}.{}",
        runway_core::slugify::branch_slug(&deployment.branch),
        ctx.settings.deploy_domain
    );
    let url = format!("{}://{}", ctx.settings.url_scheme, host);
    let secs = (chrono::Utc::now() - deployment.created_at)
        .num_seconds()
        .max(0);
    let dur = format!("{}m{:02}s", secs / 60, secs % 60);
    let state_line = if outcome == "ready" {
        format!("**ready** in {dur}")
    } else {
        format!("**failed** after {dur}")
    };
    let body = format!(
        "**Runway preview** — [{url}]({url})\n\n`{}` @ `{}` — {state_line}.",
        deployment.branch,
        &deployment.commit_sha[..deployment.commit_sha.len().min(7)],
    );
    let marker = format!("<!-- runway-preview:{} -->", project.id);
    if let Err(e) = gh
        .upsert_issue_comment(&token, &project.repo_full_name, pr_number, &marker, &body)
        .await
    {
        tracing::warn!(deployment_id = deployment.id, error = %e, "preview comment update failed");
    }
}

/// Resolve the daemon a deployment's container lives on.
async fn dep_docker(ctx: &Ctx, remote_node_id: Option<&str>) -> Option<bollard::Docker> {
    match remote_node_id {
        Some(nid) => dkr::node_client(&ctx.db, nid, &ctx.crypto).await,
        None => Some(ctx.docker.clone()),
    }
}

/// `delete_container` — remove a stopped deployment container + edge network.
pub async fn delete_container(ctx: &Ctx, deployment_id: &str) -> anyhow::Result<()> {
    let Some(deployment) = deploy::get(&ctx.db, deployment_id).await? else {
        return Ok(());
    };
    let Some(docker) = dep_docker(ctx, deployment.remote_node_id.as_deref()).await else {
        // Node unreachable — leave the row for a later retry.
        tracing::warn!(deployment_id, "remote node unreachable, delete deferred");
        return Ok(());
    };

    // Remote static deploys leave their artifact volume on the node. It can
    // only be removed after the serve container is gone — a mounted volume
    // cannot be deleted while still in use.
    let static_volume = (deployment.remote_node_id.is_some() && deployment.is_static())
        .then(|| format!("runway-static-{deployment_id}"));

    let Some(container_id) = &deployment.container_id else {
        if let Some(volume) = &static_volume {
            let _ = docker
                .remove_volume(volume, None::<bollard::volume::RemoveVolumeOptions>)
                .await;
        }
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
    if let Some(volume) = &static_volume {
        let _ = docker
            .remove_volume(volume, None::<bollard::volume::RemoveVolumeOptions>)
            .await;
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

    // Retention — opt-in `config.deployment_retention` keeps the newest N
    // completed deployments per environment; older rows no alias
    // references are pruned with metrics, logs, and artifacts. Rollback
    // targets (any alias's previous_deployment_id) are always spared.
    let keep = project
        .config
        .get("deployment_retention")
        .and_then(|v| v.as_i64())
        .filter(|&n| n > 0)
        .unwrap_or(i64::MAX);
    if keep < i64::MAX {
        let stale: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM (
                 SELECT id,
                        ROW_NUMBER() OVER (
                            PARTITION BY environment_id ORDER BY created_at DESC
                        ) rn
                 FROM deployment
                 WHERE project_id = $1 AND status = 'completed'
                   AND id NOT IN (
                       SELECT a.deployment_id FROM alias a
                       JOIN deployment d ON a.deployment_id = d.id
                       WHERE d.project_id = $1
                       UNION
                       SELECT a.previous_deployment_id FROM alias a
                       JOIN deployment d ON a.previous_deployment_id = d.id
                       WHERE d.project_id = $1
                   )
             ) t WHERE rn > $2",
        )
        .bind(project_id)
        .bind(keep)
        .fetch_all(&ctx.db)
        .await?;
        for (id,) in stale {
            let Some(dep) = deploy::get(&ctx.db, &id).await? else {
                continue;
            };
            drop_artifacts(ctx, &dep).await;
            sqlx::query("DELETE FROM deployment_metric WHERE deployment_id = $1")
                .bind(&dep.id)
                .execute(&ctx.db)
                .await?;
            let _ = tokio::fs::remove_file(ctx.logs.path(&dep.id)).await;
            sqlx::query("DELETE FROM deployment WHERE id = $1")
                .bind(&dep.id)
                .execute(&ctx.db)
                .await?;
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

struct ImageBuildSpec<'a> {
    dockerfile: &'a str,
    root_dir: &'a str,
    fetch_image: &'a str,
    source_cmds: &'a [String],
    env: &'a [String],
}

/// KERN executor phase 1: dispatch the generated workflow, poll to
/// conclusion, download the `runway-out` artifact into `static_local_dir`.
/// The script is the local runner's `commands.join(" && ")` with the
/// container-absolute paths rewritten to the job workspace.
#[allow(clippy::too_many_arguments)]
async fn kern_static_build<F, Fut>(
    ctx: &Ctx,
    deployment: &Deployment,
    commands: &[String],
    env: &[String],
    runner_image: &str,
    source_archive: Option<&str>,
    static_local_dir: &std::path::Path,
    log: &F,
) -> anyhow::Result<()>
where
    F: Fn(&str) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use runway_core::kern;

    let (url, token, project_slug) = match (
        &ctx.settings.kern_url,
        &ctx.settings.kern_token,
        &ctx.settings.kern_project,
    ) {
        (Some(u), Some(t), Some(p)) => (u.clone(), t.clone(), p.clone()),
        _ => anyhow::bail!(
            "build_executor=kern but RUNWAY_KERN_URL/RUNWAY_KERN_TOKEN/RUNWAY_KERN_PROJECT unset"
        ),
    };
    let client = kern::KernClient::new(&url, &token, &project_slug);

    // Source delivery: upload deploys ship the deployment tarball itself;
    // git deploys ship a marker bundle. Either way the bundle replaces
    // KERN's repo checkout — the build script then does the real fetch.
    let mut cmds = commands.to_vec();
    // Local `/out` is a mount outside the workspace; on KERN `runway-out`
    // lives inside it, so `output_directory=.` would copy into itself.
    // Swap the publish command for a self-excluding tar pipeline.
    let output_dir = deployment
        .config
        .get("output_directory")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_matches('/')
        .to_string();
    for c in cmds.iter_mut() {
        if c.contains("Publishing static output") {
            *c = format!(
                "test -f $GITHUB_WORKSPACE/.runway-src-ok || {{ printf '\\033[31mError: source acquisition failed\\033[0m\\n' 1>&2; exit 1; }}; \
                 OUTDIR=$(readcfg output_directory); \
                 OUTDIR=${{OUTDIR:-{output_dir}}}; \
                 echo \"Publishing static output ($OUTDIR)...\"; \
                 test -d \"$OUTDIR\" || {{ printf '\\033[31mError: output directory %s not found\\033[0m\\n' \"$OUTDIR\" 1>&2; exit 1; }}; \
                 mkdir -p \"$GITHUB_WORKSPACE/runway-out\" && \
                 tar -cf - --exclude='./runway-out' -C \"$OUTDIR\" . | tar -xf - -C \"$GITHUB_WORKSPACE/runway-out\" && \
                 test -n \"$(find \"$GITHUB_WORKSPACE/runway-out\" -type f -print -quit)\" || {{ printf '\\033[31mError: build produced no files\\033[0m\\n' 1>&2; exit 1; }} && \
                 (cp runway.json \"$GITHUB_WORKSPACE/runway-out/.runway.json\" 2>/dev/null || true) && \
                 echo 'Static output published'"
            );
        }
    }
    let bundle: Vec<u8> = if let Some(archive) = source_archive {
        // The bundle lands pre-extracted in the job workspace, so the
        // tarball-extract command collapses to its marker.
        cmds[0] = "touch .runway-src-ok".into();
        let path = std::path::Path::new(&ctx.settings.data_dir)
            .join("uploads")
            .join(archive);
        tokio::fs::read(&path).await?
    } else {
        let tmp = static_local_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!(".kern-src-{}", deployment.id));
        tokio::fs::create_dir_all(&tmp).await?;
        tokio::fs::write(tmp.join(".runway-marker"), b"runway\n").await?;
        let out = tokio::process::Command::new("tar")
            .arg("-czf")
            .arg(tmp.join("bundle.tgz"))
            .arg("-C")
            .arg(&tmp)
            .arg(".runway-marker")
            .output()
            .await?;
        if !out.status.success() {
            anyhow::bail!(
                "marker bundle failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let bytes = tokio::fs::read(tmp.join("bundle.tgz")).await?;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        bytes
    };
    let bundle_ref = client.upload_source(&bundle).await?;
    let script = cmds
        .join(" && ")
        .replace("/app", "$GITHUB_WORKSPACE")
        .replace("/out", "$GITHUB_WORKSPACE/runway-out");
    let script = format!("cd \"$GITHUB_WORKSPACE\" && {script}");

    let labels: Vec<String> = deployment
        .config
        .get("kern_labels")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["self-hosted".into(), "linux".into(), "docker".into()]);
    let env_pairs: Vec<(String, String)> = env
        .iter()
        .filter_map(|e| e.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    log(&format!(
        "Dispatching build to KERN ({project_slug}, labels: {})...",
        labels.join(", ")
    ))
    .await;
    let yaml = kern::workflow_yaml(&script, runner_image, &labels, &env_pairs);
    let run_id = client
        .dispatch(
            &yaml,
            &deployment.branch,
            &deployment.commit_sha,
            &bundle_ref,
        )
        .await?;
    log(&format!("KERN run dispatched: {run_id}")).await;
    // Record for ops/inspection and the cancel path.
    let _ = sqlx::query("UPDATE deployment SET config = config || $1::jsonb WHERE id = $2")
        .bind(json!({ "kern_run_id": run_id }))
        .bind(&deployment.id)
        .execute(&ctx.db)
        .await;

    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(ctx.settings.deployment_timeout_seconds);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if std::time::Instant::now() > deadline {
            let _ = client.cancel(&run_id).await;
            anyhow::bail!("kern run timed out");
        }
        let local: Option<String> =
            sqlx::query_scalar("SELECT conclusion FROM deployment WHERE id = $1")
                .bind(&deployment.id)
                .fetch_optional(&ctx.db)
                .await?
                .flatten();
        if local.as_deref() == Some("canceled") {
            let _ = client.cancel(&run_id).await;
            anyhow::bail!("deployment canceled");
        }
        let run = client.run(&run_id).await?;
        if run.status.as_deref() == Some("completed") {
            match run.conclusion.as_deref() {
                Some("success") => break,
                other => anyhow::bail!("kern run concluded: {}", other.unwrap_or("failure")),
            }
        }
    }
    log("KERN build finished; importing artifact...").await;

    let artifacts = client.artifacts(&run_id).await?;
    let artifact = artifacts
        .iter()
        .find(|a| a.name == "runway-out")
        .ok_or_else(|| anyhow::anyhow!("kern run produced no runway-out artifact"))?;
    let bytes = client.download(&artifact.id).await?;

    // upload-artifact archives the dir as runway-out/… — flatten onto the
    // static dir so `.runway.json` and assets land where phase 2 expects.
    let tmp = static_local_dir
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(".kern-{}", deployment.id));
    tokio::fs::create_dir_all(&tmp).await?;
    // Keep the archive a sibling of the extract root — inside tmp it would
    // be copied into the served tree along with the payload.
    let tgz = tmp.with_extension("tgz");
    tokio::fs::write(&tgz, &bytes).await?;
    let untar = tokio::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tgz)
        .arg("-C")
        .arg(&tmp)
        .output()
        .await?;
    if !untar.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        anyhow::bail!(
            "kern artifact extract failed: {}",
            String::from_utf8_lossy(&untar.stderr)
        );
    }
    let root = if tmp.join("runway-out").is_dir() {
        tmp.join("runway-out")
    } else {
        tmp.clone()
    };
    let copy = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cp -a '{}'/. '{}/'",
            root.display(),
            static_local_dir.display()
        ))
        .output()
        .await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let _ = tokio::fs::remove_file(&tgz).await;
    if !copy.status.success() {
        anyhow::bail!(
            "kern artifact copy failed: {}",
            String::from_utf8_lossy(&copy.stderr)
        );
    }
    Ok(())
}

/// `dockerfile_path` builds: a throwaway fetch container stages the repo
/// (`buildctx/<dep>` mounted at /app locally, pulled back over the
/// archive API on remote nodes), then `docker build` runs with that
/// directory as a real context — devpush built with no context, so
/// `COPY` could never work there. Returns the built tag, which lives on
/// whichever daemon ran the build.
async fn dockerfile_build<F, Fut>(
    ctx: &Ctx,
    docker: &bollard::Docker,
    deployment: &Deployment,
    log: &F,
    spec: ImageBuildSpec<'_>,
) -> anyhow::Result<String>
where
    F: Fn(&str) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let ImageBuildSpec {
        dockerfile: dockerfile_path,
        root_dir,
        fetch_image,
        source_cmds,
        env,
    } = spec;
    let deployment_id = &deployment.id;
    let remote = deployment.remote_node_id.is_some();
    let ctx_local = std::path::Path::new(&ctx.settings.data_dir)
        .join("buildctx")
        .join(deployment_id);
    let _ = tokio::fs::remove_dir_all(&ctx_local).await;
    tokio::fs::create_dir_all(&ctx_local).await?;

    log("Staging source for image build...").await;
    if docker.inspect_image(fetch_image).await.is_err() {
        log(&format!("Pulling fetch image ({fetch_image})...")).await;
        pull_image(docker, fetch_image).await?;
    }
    // Remote nodes can't see local host paths: the fetch container stages
    // the repo inside its own layer and we pull it back as an archive.
    let mut fetch_binds = vec![];
    if !remote {
        let ctx_host = docker_host_root(&ctx.settings)?
            .join("buildctx")
            .join(deployment_id);
        fetch_binds.push(format!("{}:/app", ctx_host.display()));
        let host_root = docker_host_root(&ctx.settings)?;
        if deployment
            .config
            .get("source_archive")
            .and_then(|v| v.as_str())
            .is_some()
        {
            fetch_binds.push(format!("{}:/src:ro", host_root.join("uploads").display()));
        }
    }
    let fetch_name = format!(
        "runner-{}-fetch",
        &deployment_id[..7.min(deployment_id.len())]
    );
    let fetch_body = ContainerConfig {
        image: Some(fetch_image.to_string()),
        env: Some(env.to_vec()),
        working_dir: Some("/app".into()),
        labels: Some(HashMap::from([
            ("runway.deployment_id".into(), deployment_id.clone()),
            ("runway.project_id".into(), deployment.project_id.clone()),
            ("runway.role".into(), "fetch".into()),
        ])),
        host_config: Some(HostConfig {
            binds: Some(fetch_binds),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            ..Default::default()
        }),
        // alpine/git's ENTRYPOINT is `git` — replace it, else our
        // /bin/sh -c script becomes git argv and dies instantly.
        entrypoint: Some(vec!["/bin/sh".into(), "-c".into()]),
        cmd: Some(vec![source_cmds.join(" && ")]),
        ..Default::default()
    };
    let fetch_id = dkr::create_or_replace_container(docker, &fetch_name, fetch_body)
        .await
        .map_err(|e| anyhow::anyhow!("fetch container: {e}"))?;
    docker
        .start_container(&fetch_id, None::<StartContainerOptions<String>>)
        .await?;
    spawn_log_tailer(ctx, fetch_id.clone(), deployment_id.clone());
    let exit = wait_exit(docker, &fetch_id, ctx.settings.deployment_timeout_seconds).await;
    if remote && matches!(exit, Ok(0)) {
        // The repo lives in the dead fetch container's layer — pull it
        // back as `app/` under ctx_local (same archive-API path as the
        // remote static artifact return).
        if let Err(e) = download_container_dir(docker, &fetch_id, "/app", &ctx_local).await {
            let _ = docker
                .remove_container(
                    &fetch_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
            anyhow::bail!("remote source fetch: {e}");
        }
    }
    let _ = docker
        .remove_container(
            &fetch_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    match exit {
        Ok(0) => {}
        Ok(code) => anyhow::bail!("source fetch failed (exit code {code})"),
        Err(e) => anyhow::bail!("source fetch failed: {e}"),
    }

    // `root_directory` selects the context root — a monorepo app builds
    // with only its subtree visible, matching Railway/Coolify behavior.
    let staged_dir = ctx_local.clone();
    let context_dir = if root_dir.is_empty() {
        staged_dir
    } else {
        staged_dir.join(root_dir)
    };
    if !context_dir.join(dockerfile_path).is_file() {
        anyhow::bail!("Dockerfile not found at {dockerfile_path}");
    }

    log(&format!("Building image ({dockerfile_path})...")).await;
    let tag = format!(
        "runway-dep-{}:latest",
        &deployment_id[..12.min(deployment_id.len())]
    );
    let buildargs: HashMap<String, String> = deployment
        .config
        .get("build_args")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let tar = tar_context(&context_dir).await?;
    let mut stream = docker.build_image(
        bollard::image::BuildImageOptions::<String> {
            dockerfile: dockerfile_path.to_string(),
            t: tag.clone(),
            rm: true,
            forcerm: true,
            buildargs,
            labels: HashMap::from([
                ("runway.deployment_id".into(), deployment_id.clone()),
                ("runway.project_id".into(), deployment.project_id.clone()),
            ]),
            ..Default::default()
        },
        None,
        Some(tar.into()),
    );
    // jarvis: build runs to completion — cancel() can't abort a daemon-side
    // build stream. Upgrade if large builds need mid-flight cancel.
    let collect = async {
        while let Some(item) = stream.next().await {
            let info = item.map_err(|e| anyhow::anyhow!("image build stream: {e}"))?;
            if let Some(txt) = info.stream {
                for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                    log(line).await;
                }
            }
            if let Some(err) = info.error {
                anyhow::bail!("image build failed: {err}");
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(ctx.settings.deployment_timeout_seconds.max(300)),
        collect,
    )
    .await
    .map_err(|_| anyhow::anyhow!("image build timed out"))??;

    let _ = tokio::fs::remove_dir_all(&ctx_local).await;
    log(&format!("Image built ({tag})")).await;
    Ok(tag)
}

/// Tar a directory for `docker build` context. `.git` is excluded.
// jarvis: .dockerignore not honored — ceiling until a real ignore parser.
async fn tar_context(dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let out = tokio::process::Command::new("tar")
        .args(["--exclude=./.git", "-cf", "-", "-C"])
        .arg(dir)
        .arg(".")
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!(
            "tar context failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(out.stdout)
}

/// Recursive byte total of a directory (usage metering).
fn walkdir_size(dir: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if let Ok(m) = e.metadata() {
                    if m.is_dir() {
                        stack.push(p);
                    } else {
                        total += m.len();
                    }
                }
            }
        }
    }
    total
}

/// Repo-relative path check for dockerfile_path — no absolute, no `..`.
fn safe_rel_path(p: &str) -> bool {
    use std::path::Component;
    !p.is_empty()
        && std::path::Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Remove host-side deployment artifacts: static output dir, upload
/// tarball, dockerfile build context + built image.
/// The upload tarball is retained until this point because `redeploy` of an
/// upload deployment re-extracts it.
async fn drop_artifacts(ctx: &Ctx, dep: &Deployment) {
    let data = std::path::Path::new(&ctx.settings.data_dir);
    let _ = tokio::fs::remove_dir_all(data.join("static").join(&dep.id)).await;
    let _ = tokio::fs::remove_dir_all(data.join("buildctx").join(&dep.id)).await;
    let _ = ctx
        .docker
        .remove_image(
            &format!("runway-dep-{}:latest", &dep.id[..12.min(dep.id.len())]),
            None,
            None,
        )
        .await;
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

/// `project.config.node_selector` — array of label strings the remote
/// node must carry (`["arch=arm64", "gpu"]`). Pick the least-loaded
/// online, non-local node matching every selector. An explicit
/// `remote_node_id` pin always wins; absence of both means local.
async fn select_node_by_labels(
    db: &sqlx::PgPool,
    project: &Project,
) -> anyhow::Result<Option<runway_core::models::RemoteNode>> {
    let Some(selectors) = project
        .config
        .get("node_selector")
        .and_then(|v| v.as_array())
    else {
        return Ok(None);
    };
    let wanted: Vec<&str> = selectors.iter().filter_map(|s| s.as_str()).collect();
    if wanted.is_empty() {
        return Ok(None);
    }
    let nodes: Vec<runway_core::models::RemoteNode> =
        sqlx::query_as("SELECT * FROM remote_node WHERE status = 'online'")
            .fetch_all(db)
            .await?;
    let mut best: Option<(runway_core::models::RemoteNode, i64)> = None;
    for n in nodes {
        if n.is_local() {
            continue;
        }
        let labels: Vec<&str> = n
            .labels
            .as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str()).collect())
            .unwrap_or_default();
        if !wanted.iter().all(|w| labels.contains(w)) {
            continue;
        }
        let active: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM deployment
             WHERE remote_node_id = $1
               AND status NOT IN ('completed','failed','canceled')",
        )
        .bind(&n.id)
        .fetch_one(db)
        .await?;
        if best.as_ref().is_none_or(|(_, c)| active.0 < *c) {
            best = Some((n, active.0));
        }
    }
    match best {
        Some((n, load)) if load < n.max_deployments as i64 => Ok(Some(n)),
        Some((n, _)) => {
            tracing::warn!(node = %n.id, "node_selector match at capacity, using local daemon");
            Ok(None)
        }
        None => Ok(None),
    }
}

/// Pull `path` from a (possibly exited) container over the archive API
/// and unpack the tar into `dest` — docker returns it as `<leaf>/…`.
async fn download_container_dir(
    docker: &bollard::Docker,
    container: &str,
    path: &str,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let mut stream = docker.download_from_container(
        container,
        Some(bollard::container::DownloadFromContainerOptions { path }),
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
    }
    let tmp = dest.join(".dl");
    tokio::fs::create_dir_all(&tmp).await?;
    let tar = tmp.with_extension("tar");
    tokio::fs::write(&tar, &bytes).await?;
    let out = tokio::process::Command::new("tar")
        .arg("-xf")
        .arg(&tar)
        .arg("-C")
        .arg(&tmp)
        .output()
        .await?;
    let _ = tokio::fs::remove_file(&tar).await;
    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        anyhow::bail!(
            "container archive extract failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Flatten `<leaf>/…` onto dest.
    let leaf = path.rsplit('/').next().unwrap_or(path);
    let cp = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cp -a '{}/{}/.' '{}/'",
            tmp.display(),
            leaf,
            dest.display()
        ))
        .output()
        .await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    if !cp.status.success() {
        anyhow::bail!(
            "container archive copy failed: {}",
            String::from_utf8_lossy(&cp.stderr)
        );
    }
    Ok(())
}

/// Remote static: pull the exited build container's /out as a tar stream
/// and unpack it into `dest` (runs before the container is removed).
async fn download_static_out(
    docker: &bollard::Docker,
    container: &str,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let mut stream = docker.download_from_container(
        container,
        Some(bollard::container::DownloadFromContainerOptions {
            path: "/app/.runway-out",
        }),
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
    }
    let parent = dest.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = parent.join(format!(".out-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&tmp).await?;
    let tar = tmp.with_extension("tar");
    tokio::fs::write(&tar, &bytes).await?;
    let out = tokio::process::Command::new("tar")
        .arg("-xf")
        .arg(&tar)
        .arg("-C")
        .arg(&tmp)
        .output()
        .await?;
    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let _ = tokio::fs::remove_file(&tar).await;
        anyhow::bail!(
            "artifact tar failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Docker archives the dir as `.runway-out/…`; flatten onto the
    // static dir.
    let root = if tmp.join(".runway-out").is_dir() {
        tmp.join(".runway-out")
    } else {
        tmp.clone()
    };
    let cp = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cp -a '{}'/. '{}/'",
            root.display(),
            dest.display()
        ))
        .output()
        .await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let _ = tokio::fs::remove_file(&tar).await;
    if !cp.status.success() {
        anyhow::bail!(
            "artifact copy failed: {}",
            String::from_utf8_lossy(&cp.stderr)
        );
    }
    Ok(())
}

/// Remote static: stage `src_dir` (+ optional SWS toml) into a named
/// volume on the node. A short-lived helper container receives the tar
/// at the volume mount — upload_to_container is the only file channel
/// the remote daemon API exposes.
async fn ship_static_volume(
    docker: &bollard::Docker,
    volume: &str,
    src_dir: &std::path::Path,
    toml: Option<&str>,
    runner_image: &str,
) -> anyhow::Result<()> {
    docker
        .create_volume(bollard::volume::CreateVolumeOptions {
            name: volume,
            ..Default::default()
        })
        .await?;

    let parent = src_dir
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let stage = parent.join(format!(".vol-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(stage.join("public")).await?;
    let cp = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cp -a '{}'/. '{}/public/'",
            src_dir.display(),
            stage.display()
        ))
        .output()
        .await?;
    if !cp.status.success() {
        let _ = tokio::fs::remove_dir_all(&stage).await;
        anyhow::bail!(
            "artifact staging failed: {}",
            String::from_utf8_lossy(&cp.stderr)
        );
    }
    if let Some(toml) = toml {
        tokio::fs::create_dir_all(stage.join("_cfg")).await?;
        tokio::fs::write(stage.join("_cfg").join("config.toml"), toml).await?;
    }
    let tar = stage.with_extension("tar");
    let mk = tokio::process::Command::new("tar")
        .arg("-cf")
        .arg(&tar)
        .arg("-C")
        .arg(&stage)
        .arg(".")
        .output()
        .await?;
    if !mk.status.success() {
        let _ = tokio::fs::remove_dir_all(&stage).await;
        anyhow::bail!(
            "artifact tar failed: {}",
            String::from_utf8_lossy(&mk.stderr)
        );
    }
    let payload = tokio::fs::read(&tar).await?;
    let _ = tokio::fs::remove_dir_all(&stage).await;
    let _ = tokio::fs::remove_file(&tar).await;

    let helper = format!("runway-ship-{}", &volume[volume.len().saturating_sub(12)..]);
    let id = dkr::create_or_replace_container(
        docker,
        &helper,
        ContainerConfig {
            image: Some(runner_image.to_string()),
            cmd: Some(vec!["sh".into(), "-c".into(), "sleep 600".into()]),
            host_config: Some(HostConfig {
                mounts: Some(vec![bollard::models::Mount {
                    target: Some("/srv".into()),
                    source: Some(volume.to_string()),
                    typ: Some(bollard::models::MountTypeEnum::VOLUME),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await?;
    docker
        .start_container(&id, None::<StartContainerOptions<String>>)
        .await?;
    let shipped = docker
        .upload_to_container(
            &id,
            Some(bollard::container::UploadToContainerOptions {
                path: "/srv",
                ..Default::default()
            }),
            payload.into(),
        )
        .await;
    let _ = docker
        .remove_container(
            &id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    shipped?;
    Ok(())
}
