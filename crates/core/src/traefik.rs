//! Traefik dynamic file provider writer.
//!
//! The app emits `<data_dir>/traefik/project_<id>.yml`; Traefik watches
//! the directory. Writes are atomic (temp + rename) so Traefik never
//! reads a partial config. Port of DeploymentService.update_traefik_config.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sqlx::PgPool;

use crate::config::Settings;
use crate::error::Result;
use crate::models::{Domain, Project};

pub fn config_path(data_dir: &str, project_id: &str) -> PathBuf {
    Path::new(data_dir)
        .join("traefik")
        .join(format!("project_{project_id}.yml"))
}

/// Write a YAML config atomically (temp + rename).
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("yml.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Rebuild the per-project Traefik config from aliases + active domains.
/// `include_deployment_ids` lets a just-finalized deployment appear in the
/// config before its `succeeded` conclusion is visible to other queries.
pub async fn update_project_config(
    db: &PgPool,
    project: &Project,
    settings: &Settings,
    include_deployment_ids: &[String],
) -> Result<()> {
    let path = config_path(&settings.data_dir, &project.id);

    // Aliases pointing at succeeded (or explicitly included) deployments.
    // Remote deployments carry node host + published port → a file-provider
    // loadBalancer service instead of the `@docker` docker-provider service.
    type AliasRow = (
        String,
        String,
        String,
        Option<String>,
        i64,
        Option<String>,
        Option<i32>,
        Option<String>,
    );
    let aliases: Vec<AliasRow> = if include_deployment_ids.is_empty() {
        sqlx::query_as(
            "SELECT a.subdomain, a.deployment_id, a.type, a.value, a.id,
                    d.remote_node_id, d.remote_port, n.host
             FROM alias a JOIN deployment d ON a.deployment_id = d.id
             LEFT JOIN remote_node n ON n.id = d.remote_node_id
             WHERE d.project_id = $1 AND d.conclusion = 'succeeded'",
        )
        .bind(&project.id)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as(
            "SELECT a.subdomain, a.deployment_id, a.type, a.value, a.id,
                    d.remote_node_id, d.remote_port, n.host
             FROM alias a JOIN deployment d ON a.deployment_id = d.id
             LEFT JOIN remote_node n ON n.id = d.remote_node_id
             WHERE d.project_id = $1
               AND (d.conclusion = 'succeeded' OR d.id = ANY($2))",
        )
        .bind(&project.id)
        .bind(include_deployment_ids)
        .fetch_all(db)
        .await?
    };

    let domains: Vec<Domain> =
        sqlx::query_as("SELECT * FROM domain WHERE project_id = $1 AND status = 'active'")
            .bind(&project.id)
            .fetch_all(db)
            .await?;

    // Nothing to route → drop the file entirely.
    if aliases.is_empty() && domains.is_empty() {
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        return Ok(());
    }

    let mut routers = serde_json::Map::new();
    let mut middlewares = serde_json::Map::new();
    let https = settings.url_scheme == "https";
    let entry_points = if https {
        json!(["web", "websecure"])
    } else {
        json!(["web"])
    };

    let mut services = serde_json::Map::new();

    // Router service reference: remote deployments get a file-provider
    // loadBalancer to `node.host:remote_port`; local ones keep the
    // docker-provider service created from container labels.
    let service_ref = |deployment_id: &str,
                       remote: &Option<String>,
                       port: Option<i32>,
                       host: &Option<String>,
                       services: &mut serde_json::Map<String, Value>|
     -> String {
        let name = format!("deployment-{deployment_id}");
        if let (Some(_node), Some(port), Some(host)) = (remote, port, host) {
            services.entry(name.clone()).or_insert_with(|| {
                json!({
                    "loadBalancer": {
                        "servers": [{ "url": format!("http://{host}:{port}") }]
                    }
                })
            });
            name
        } else {
            format!("{name}@docker")
        }
    };

    for (subdomain, deployment_id, _ty, _value, alias_id, rnode, rport, rhost) in &aliases {
        let svc = service_ref(deployment_id, rnode, *rport, rhost, &mut services);
        let mut router = json!({
            "rule": format!("Host(`{}.{}`)", subdomain, settings.deploy_domain),
            "service": svc,
            "entryPoints": entry_points,
        });
        if https {
            router["tls"] = json!({ "certResolver": "le" });
        }
        routers.insert(format!("router-alias-{alias_id}"), router);
    }

    for domain in &domains {
        // Domains route to the current deployment of their environment alias.
        let env_alias = aliases.iter().find(|(_, _, ty, value, _, _, _, _)| {
            ty == "environment_id" && value.as_deref() == domain.environment_id.as_deref()
        });
        let Some((_, deployment_id, _, _, _, rnode, rport, rhost)) = env_alias else {
            continue;
        };

        if domain.r#type == "route" {
            let svc = service_ref(deployment_id, rnode, *rport, rhost, &mut services);
            let mut router = json!({
                "rule": format!("Host(`{}`)", domain.hostname),
                "service": svc,
                "entryPoints": entry_points,
            });
            if https {
                // lehttp: custom domains use the HTTP-01 resolver.
                router["tls"] = json!({ "certResolver": "lehttp" });
            }
            routers.insert(format!("router-domain-{}", domain.id), router);
        } else {
            // 301/302/307/308 → redirect middleware to the env hostname.
            let mw = format!("redirect-{}", domain.id);
            let target = env_hostname_for(domain.environment_id.as_deref(), project, settings);
            let mut router = json!({
                "rule": format!("Host(`{}`)", domain.hostname),
                "service": "noop@internal",
                "middlewares": [mw],
                "entryPoints": entry_points,
            });
            if https {
                router["tls"] = json!({ "certResolver": "lehttp" });
            }
            routers.insert(format!("router-redirect-{}", domain.id), router);
            middlewares.insert(
                mw.clone(),
                json!({
                    "redirectRegex": {
                        "regex": format!("^https?://{}/(.*)", domain.hostname),
                        "replacement": format!("{}://{}/${{1}}", settings.url_scheme, target),
                        "permanent": domain.r#type == "301",
                    }
                }),
            );
        }
    }

    // Path-level redirect rules → redirectRegex middlewares attached to
    // every router serving this project (devpush parity).
    let rules: Vec<(String, String, String, i32)> = sqlx::query_as(
        "SELECT id, source_path, target_url, status_code
         FROM redirect_rule WHERE project_id = $1 AND enabled",
    )
    .bind(&project.id)
    .fetch_all(db)
    .await?;
    for (rule_id, source_path, target_url, status_code) in &rules {
        let mw = format!("redirect-rule-{rule_id}");
        middlewares.insert(
            mw.clone(),
            json!({
                "redirectRegex": {
                    // Traefik matches the full URL; anchor past the
                    // host so `/old` matches `https://h/old`. devpush's
                    // `^/path` never matches a full URL — fixed here.
                    "regex": format!("^https?://[^/]+{}(.*)", regex_escape(source_path)),
                    "replacement": format!("{target_url}$1"),
                    "permanent": matches!(status_code, 301 | 308),
                }
            }),
        );
        for router in routers.values_mut() {
            if router["service"]
                .as_str()
                .is_some_and(|s| s.starts_with("deployment-"))
            {
                if let Some(arr) = router["middlewares"].as_array_mut() {
                    arr.push(json!(mw.clone()));
                } else {
                    router["middlewares"] = json!([mw.clone()]);
                }
            }
        }
    }

    let doc = json!({
        "http": {
            "routers": routers,
            "middlewares": middlewares,
            "services": services,
        }
    });
    let yaml = serde_yaml::to_string(&doc)
        .map_err(|e| crate::error::Error::Config(format!("traefik yaml: {e}")))?;
    write_atomic(&path, &yaml)
}

/// Hostname an environment's domain redirects/routes to.
fn env_hostname_for(env_id: Option<&str>, project: &Project, settings: &Settings) -> String {
    match env_id {
        Some("prod") | None => project.hostname(settings),
        Some(id) => {
            let slug = project
                .environment_by_id(id)
                .map(|e| e.slug)
                .unwrap_or_else(|| "production".into());
            project.environment_hostname(&slug, settings)
        }
    }
}

/// Escape regex metacharacters in a literal path (devpush `re.escape`).
fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | ' ' | '~') {
                c.to_string().chars().collect::<Vec<_>>()
            } else {
                vec!['\\', c]
            }
        })
        .collect()
}

/// Delete a project's dynamic config entirely.
pub async fn remove_project_config(project_id: &str, settings: &Settings) -> Result<()> {
    let path = config_path(&settings.data_dir, project_id);
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}
