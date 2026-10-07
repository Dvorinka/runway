//! Storage provisioning — port of devpush `workers/tasks/storage.py`.
//!
//! Engines: sqlite (file only), postgres (`postgres:16-alpine`),
//! mongodb (`mongo:7`), redis kv (`redis:7-alpine --requirepass`),
//! volume (directory only). Containers `storage-<id12>` on a dedicated
//! network `runway_storage_<id12>`; data persists under
//! `data/storage/<team>/<type>/<name>` bound to `/data`.
//!
//! Deviation from devpush: `PGDATA=/data/pg` is set so the postgres
//! bind mount actually holds the data (devpush forgot it → data went
//! to the anonymous volume). Password stored AES-GCM in
//! `config.password_enc`, not plaintext.

use std::collections::HashMap;

use bollard::container::{Config, NetworkingConfig, StartContainerOptions};
use bollard::models::{EndpointSettings, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use serde_json::{json, Value};
use sqlx::PgPool;

use runway_core::docker as dkr;
use runway_core::models::Storage;

use crate::deploy::docker_host_root;
use crate::Ctx;

/// Delete a storage data dir that may contain container-owned files
/// (postgres writes as uid 70 inside the bind — plain remove_dir_all
/// hits EACCES). Wipe contents via an ephemeral container with the dir
/// bound, then remove the empty dir from the host. `image` must already
/// be pulled — pass the engine's own image.
/// Image guaranteed present for wiping — the engine's own (alpine for
/// file-only storage, which may pull once).
fn wipe_image(storage: &Storage) -> &'static str {
    match (storage.r#type.as_str(), storage.engine()) {
        ("database", "postgres") => "postgres:16-alpine",
        ("database", "mongodb") => "mongo:7",
        ("kv", _) => "redis:7-alpine",
        _ => "alpine:3",
    }
}

async fn remove_storage_dir(ctx: &Ctx, local_path: &std::path::Path, image: &str) {
    // Bind source must be host-visible (HOST_DATA_DIR when runway is
    // itself containerized); removal runs on the local fs path.
    let host_path = docker_host_root(&ctx.settings).ok().map(|root| {
        local_path
            .strip_prefix(&ctx.settings.data_dir)
            .map(|rel| root.join(rel))
            .unwrap_or_else(|_| local_path.to_path_buf())
    });
    let Some(host_path) = host_path else {
        let _ = tokio::fs::remove_dir_all(local_path).await;
        return;
    };
    let body = Config {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "sh".into(),
            "-c".into(),
            "rm -rf /w/* /w/.[!.]* /w/..?*".into(),
        ]),
        user: Some("0".into()),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:/w", host_path.display())]),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    match ctx
        .docker
        .create_container(
            None::<bollard::container::CreateContainerOptions<String>>,
            body,
        )
        .await
    {
        Ok(c) => {
            let id = c.id;
            let _ = ctx
                .docker
                .start_container(&id, None::<StartContainerOptions<String>>)
                .await;
            let _ = crate::deploy::wait_exit(&ctx.docker, &id, 60).await;
        }
        Err(e) => tracing::warn!(error = %e, "storage wipe container failed"),
    }
    let _ = tokio::fs::remove_dir_all(local_path).await;
}

/// Shallow-merge object fields into `base` (serde_json has no merge).
fn merge_cfg(base: &mut Value, extra: Value) {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
}

async fn get(db: &PgPool, id: &str) -> anyhow::Result<Option<Storage>> {
    Ok(sqlx::query_as("SELECT * FROM storage WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?)
}

async fn set_status(
    db: &PgPool,
    id: &str,
    status: &str,
    config: Option<Value>,
    error: Option<Value>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE storage SET status = $1,
         config = COALESCE($2, config), error = $3, updated_at = now()
         WHERE id = $4",
    )
    .bind(status)
    .bind(config)
    .bind(error)
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

/// `provision_storage` job.
pub async fn provision(ctx: &Ctx, storage_id: &str) -> anyhow::Result<()> {
    let Some(storage) = get(&ctx.db, storage_id).await? else {
        anyhow::bail!("storage {storage_id} not found");
    };
    let res = match storage.r#type.as_str() {
        "database" => match storage.engine() {
            "sqlite" => ensure_db_path(ctx, &storage, true).await,
            e @ ("postgres" | "mongodb") => provision_docker_db(ctx, &storage, e).await,
            e => Err(anyhow::anyhow!("unsupported database engine: {e}")),
        },
        "volume" => ensure_volume_path(ctx, &storage, false).await.map(|_| ()),
        "kv" => provision_kv(ctx, &storage).await,
        t => Err(anyhow::anyhow!("unsupported storage type: {t}")),
    };
    match res {
        Ok(()) => set_status(&ctx.db, storage_id, "active", None, None).await,
        Err(e) => {
            set_status(
                &ctx.db,
                storage_id,
                "error",
                None,
                Some(json!({ "stage": "provision", "message": e.to_string() })),
            )
            .await?;
            Err(e)
        }
    }
}

/// `deprovision_storage` job — container+network+data dir, then row.
pub async fn deprovision(ctx: &Ctx, storage_id: &str) -> anyhow::Result<()> {
    let Some(storage) = get(&ctx.db, storage_id).await? else {
        anyhow::bail!("storage {storage_id} not found");
    };
    if matches!(storage.r#type.as_str(), "database" | "kv") && !matches!(storage.engine(), "sqlite")
    {
        teardown_docker(ctx, &storage).await;
    }
    let dir = std::path::PathBuf::from(storage.data_dir(&ctx.settings.data_dir));
    if dir.exists() {
        remove_storage_dir(ctx, &dir, wipe_image(&storage)).await;
    }
    sqlx::query("DELETE FROM storage_project WHERE storage_id = $1")
        .bind(storage_id)
        .execute(&ctx.db)
        .await?;
    sqlx::query("DELETE FROM storage WHERE id = $1")
        .bind(storage_id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

/// `reset_storage` job — wipe + re-provision (devpush parity).
pub async fn reset(ctx: &Ctx, storage_id: &str) -> anyhow::Result<()> {
    let Some(storage) = get(&ctx.db, storage_id).await? else {
        anyhow::bail!("storage {storage_id} not found");
    };
    set_status(&ctx.db, storage_id, "resetting", None, None).await?;
    let res = async {
        match storage.r#type.as_str() {
            "database" if storage.engine() == "sqlite" => {
                let dir = storage.data_dir(&ctx.settings.data_dir);
                tokio::fs::remove_dir_all(&dir).await?;
                ensure_db_path(ctx, &storage, true).await
            }
            "database" => {
                teardown_docker(ctx, &storage).await;
                let dir = std::path::PathBuf::from(storage.data_dir(&ctx.settings.data_dir));
                if dir.exists() {
                    remove_storage_dir(ctx, &dir, wipe_image(&storage)).await;
                }
                provision_docker_db(ctx, &storage, storage.engine()).await
            }
            "volume" => {
                let dir = storage.data_dir(&ctx.settings.data_dir);
                tokio::fs::remove_dir_all(&dir).await?;
                ensure_volume_path(ctx, &storage, false).await?;
                Ok(())
            }
            "kv" => {
                teardown_docker(ctx, &storage).await;
                let dir = std::path::PathBuf::from(storage.data_dir(&ctx.settings.data_dir));
                if dir.exists() {
                    remove_storage_dir(ctx, &dir, wipe_image(&storage)).await;
                }
                provision_kv(ctx, &storage).await
            }
            t => anyhow::bail!("unsupported storage type: {t}"),
        }
    }
    .await;
    match res {
        Ok(()) => set_status(&ctx.db, storage_id, "active", None, None).await,
        Err(e) => {
            set_status(
                &ctx.db,
                storage_id,
                "error",
                None,
                Some(json!({ "stage": "reset", "message": e.to_string() })),
            )
            .await?;
            Err(e)
        }
    }
}

// -- Provisioners --------------------------------------------------------------

async fn ensure_db_path(ctx: &Ctx, storage: &Storage, touch: bool) -> anyhow::Result<()> {
    let dir = storage.data_dir(&ctx.settings.data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    if touch {
        // Ensure the file exists so the bind never surprises sqlite.
        let db_path = format!("{dir}/db.sqlite");
        if !std::path::Path::new(&db_path).exists() {
            tokio::fs::write(&db_path, b"").await?;
        }
    }
    let mut cfg = storage.config.clone();
    cfg["path"] = json!(format!("{dir}/db.sqlite"));
    set_status(&ctx.db, &storage.id, "pending", Some(cfg), None).await?;
    Ok(())
}

async fn ensure_volume_path(ctx: &Ctx, storage: &Storage, _wipe: bool) -> anyhow::Result<()> {
    let dir = storage.data_dir(&ctx.settings.data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    let mut cfg = storage.config.clone();
    cfg["path"] = json!(dir);
    set_status(&ctx.db, &storage.id, "pending", Some(cfg), None).await?;
    Ok(())
}

async fn provision_docker_db(ctx: &Ctx, storage: &Storage, engine: &str) -> anyhow::Result<()> {
    let (image, port, env): (&str, i64, Vec<(&str, String)>) = match engine {
        "postgres" => (
            "postgres:16-alpine",
            5432,
            vec![
                ("POSTGRES_USER", "runway".into()),
                ("POSTGRES_DB", storage.name.clone()),
                // Bind is /data — tell postgres to actually use it.
                ("PGDATA", "/data/pg".into()),
            ],
        ),
        "mongodb" => (
            "mongo:7",
            27017,
            vec![
                ("MONGO_INITDB_ROOT_USERNAME", "runway".into()),
                ("MONGO_INITDB_DATABASE", storage.name.clone()),
            ],
        ),
        e => anyhow::bail!("unsupported database engine: {e}"),
    };
    let password = runway_core::slugify::token_hex(24);
    let mut env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    match engine {
        "postgres" => env.push(format!("POSTGRES_PASSWORD={password}")),
        "mongodb" => env.push(format!("MONGO_INITDB_ROOT_PASSWORD={password}")),
        _ => {}
    }
    start_container(ctx, storage, image, env, &[]).await?;

    let mut cfg = storage.config.clone();
    merge_cfg(
        &mut cfg,
        json!({
            "engine": engine,
            "container_name": storage.container_name(),
            "network_name": storage.network_name(),
            "host": storage.container_name(),
            "port": port,
            "username": "runway",
            "password_enc": ctx.crypto.encrypt(&password)?,
            "database": storage.name,
        }),
    );
    sqlx::query("UPDATE storage SET config = $1, updated_at = now() WHERE id = $2")
        .bind(&cfg)
        .bind(&storage.id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

async fn provision_kv(ctx: &Ctx, storage: &Storage) -> anyhow::Result<()> {
    let password = runway_core::slugify::token_hex(24);
    let env = vec![];
    let cmd = vec![
        "redis-server".to_string(),
        "--requirepass".to_string(),
        password.clone(),
        "--dir".to_string(),
        "/data".to_string(),
    ];
    start_container_with(ctx, storage, "redis:7-alpine", env, &cmd).await?;
    let mut cfg = storage.config.clone();
    merge_cfg(
        &mut cfg,
        json!({
            "engine": "redis",
            "container_name": storage.container_name(),
            "network_name": storage.network_name(),
            "host": storage.container_name(),
            "port": 6379,
            "password_enc": ctx.crypto.encrypt(&password)?,
        }),
    );
    sqlx::query("UPDATE storage SET config = $1, updated_at = now() WHERE id = $2")
        .bind(&cfg)
        .bind(&storage.id)
        .execute(&ctx.db)
        .await?;
    Ok(())
}

// -- Docker plumbing -----------------------------------------------------------

async fn start_container(
    ctx: &Ctx,
    storage: &Storage,
    image: &str,
    env: Vec<String>,
    cmd: &[String],
) -> anyhow::Result<()> {
    start_container_with(ctx, storage, image, env, cmd).await
}

async fn start_container_with(
    ctx: &Ctx,
    storage: &Storage,
    image: &str,
    env: Vec<String>,
    cmd: &[String],
) -> anyhow::Result<()> {
    let docker = &ctx.docker;
    let network = storage.network_name();
    dkr::ensure_network(
        docker,
        &network,
        HashMap::from([
            ("runway.managed".into(), "true".into()),
            ("runway.storage_id".into(), storage.id.clone()),
        ]),
    )
    .await?;
    crate::deploy::pull_image(docker, image).await?;

    let host_dir = docker_host_root(&ctx.settings)?
        .join("storage")
        .join(&storage.team_id)
        .join(match storage.r#type.as_str() {
            "database" => "database",
            "kv" => "kv",
            _ => "volume",
        })
        .join(&storage.name);
    tokio::fs::create_dir_all(&host_dir).await?;

    let mut labels = HashMap::from([
        ("runway.managed".into(), "true".into()),
        ("runway.storage_id".into(), storage.id.clone()),
        ("runway.storage_type".into(), storage.r#type.clone()),
        ("runway.team_id".into(), storage.team_id.clone()),
    ]);
    labels.insert("runway.storage_engine".into(), storage.engine().to_string());

    let body = Config {
        image: Some(image.to_string()),
        env: Some(env),
        cmd: if cmd.is_empty() {
            None
        } else {
            Some(cmd.to_vec())
        },
        labels: Some(labels),
        networking_config: Some(NetworkingConfig {
            endpoints_config: HashMap::from([(network, EndpointSettings::default())]),
        }),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:/data", host_dir.display())]),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let id = dkr::create_or_replace_container(docker, &storage.container_name(), body).await?;
    docker
        .start_container(&id, None::<StartContainerOptions<String>>)
        .await?;
    Ok(())
}

/// Stop+remove container and its network (best-effort, devpush parity).
async fn teardown_docker(ctx: &Ctx, storage: &Storage) {
    let name = storage.container_name();
    let _ = ctx
        .docker
        .stop_container(&name, None::<bollard::container::StopContainerOptions>)
        .await;
    let _ = ctx
        .docker
        .remove_container(
            &name,
            Some(bollard::container::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    let _ = ctx.docker.remove_network(&storage.network_name()).await;
}

// -- Deploy-time wiring --------------------------------------------------------

/// Mounts + networks for a project's linked storage — devpush
/// `DeploymentService.get_runtime_mounts` parity, plus the gap fix:
/// the deploy container also joins each linked storage's network so
/// `storage-<id12>` hostnames resolve (devpush never attached them).
/// Returns `(network_names, bind_mounts)`.
pub async fn linked(
    ctx: &Ctx,
    project_id: &str,
    environment_id: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let links: Vec<(String, Option<Value>)> = match sqlx::query_as(
        "SELECT sp.storage_id, sp.environment_ids FROM storage_project sp
         JOIN storage s ON s.id = sp.storage_id
         WHERE sp.project_id = $1 AND s.status NOT IN ('pending', 'deleted')",
    )
    .bind(project_id)
    .fetch_all(&ctx.db)
    .await
    {
        Ok(l) => l,
        Err(_) => return (vec![], vec![]),
    };
    if links.is_empty() {
        return (vec![], vec![]);
    }
    let host_root = docker_host_root(&ctx.settings)
        .unwrap_or_else(|_| std::path::PathBuf::from(&ctx.settings.data_dir));
    let mut nets = vec![];
    let mut mounts = vec![];
    for (sid, env_ids) in links {
        // environment_ids filter: empty/null = all environments.
        if let (Some(ids), Some(env)) = (&env_ids, environment_id) {
            if ids
                .as_array()
                .is_some_and(|a| !a.is_empty() && !a.iter().any(|v| v.as_str() == Some(env)))
            {
                continue;
            }
        }
        let Some(storage) = get(&ctx.db, &sid).await.ok().flatten() else {
            continue;
        };
        let type_dir = match storage.r#type.as_str() {
            "database" => "database",
            "kv" => "kv",
            _ => "volume",
        };
        if matches!(storage.r#type.as_str(), "database" | "volume") {
            let host = host_root
                .join("storage")
                .join(&storage.team_id)
                .join(type_dir)
                .join(&storage.name);
            mounts.push(format!(
                "{}:/data/{}/{}",
                host.display(),
                storage.r#type,
                storage.name
            ));
        }
        // Container-backed engines (postgres/mongodb/redis) get network
        // attach; sqlite/volume are file-only.
        if matches!(storage.r#type.as_str(), "kv")
            || (storage.r#type == "database" && storage.engine() != "sqlite")
        {
            nets.push(storage.network_name());
        }
    }
    (nets, mounts)
}
