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
        Option<String>,
    );
    let aliases: Vec<AliasRow> = if include_deployment_ids.is_empty() {
        sqlx::query_as(
            "SELECT a.subdomain, a.deployment_id, a.type, a.value, a.id,
                    d.remote_node_id, d.remote_port, n.host,
                    d.config->>'output_directory'
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
                    d.remote_node_id, d.remote_port, n.host,
                    d.config->>'output_directory'
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

    // Static-output deployments get CDN-grade caching at the edge:
    // fingerprinted assets immutable for a year (higher-priority router
    // matching asset extensions), HTML revalidated every load. Opt out
    // with `config.cdn_cache = false`.
    let cdn_enabled = project
        .config
        .get("cdn_cache")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let is_static = |outdir: &Option<String>| {
        cdn_enabled && outdir.as_deref().is_some_and(|s| !s.trim().is_empty())
    };
    let asset_rule = "PathRegexp(`\\.(js|mjs|css|map|png|jpe?g|gif|svg|ico|webp|avif|woff2?|ttf|otf|wasm|mp4|webm)$`)";
    let attach_cache = |routers: &mut serde_json::Map<String, Value>,
                        middlewares: &mut serde_json::Map<String, Value>,
                        key: String,
                        host: &str,
                        svc: &str,
                        router: &mut Value| {
        middlewares
            .entry(String::from("cdn-assets"))
            .or_insert_with(|| {
                json!({"headers": {"customResponseHeaders":
                    {"Cache-Control": "public, max-age=31536000, immutable"}}})
            });
        middlewares
            .entry(String::from("cdn-html"))
            .or_insert_with(|| {
                json!({"headers": {"customResponseHeaders":
                    {"Cache-Control": "public, max-age=0, must-revalidate"}}})
            });
        let mut mws: Vec<Value> = router["middlewares"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        mws.push(json!("cdn-html"));
        router["middlewares"] = json!(mws);
        let mut assets = json!({
            "rule": format!("Host(`{host}`) && {asset_rule}"),
            "service": svc,
            "middlewares": ["cdn-assets"],
            "entryPoints": entry_points,
        });
        if https {
            assets["tls"] = router["tls"].clone();
        }
        routers.insert(key, assets);
    };

    // Hosts serving this project — `/_runway-rum` on any of them is
    // routed to the API (speed-insights beacon, same origin as the site).
    let mut rum_hosts: Vec<String> = Vec::new();
    // Router keys exempt from deployment protection — the prod
    // environment alias and prod-bound custom domains stay public.
    let mut unprotected: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (subdomain, deployment_id, ty, value, alias_id, rnode, rport, rhost, outdir) in &aliases {
        let svc = service_ref(deployment_id, rnode, *rport, rhost, &mut services);
        let host = format!("{subdomain}.{}", settings.deploy_domain);
        rum_hosts.push(host.clone());
        let mut router = json!({
            "rule": format!("Host(`{host}`)"),
            "service": svc,
            "entryPoints": entry_points,
        });
        if https {
            router["tls"] = json!({ "certResolver": "le" });
        }
        if is_static(outdir) {
            attach_cache(
                &mut routers,
                &mut middlewares,
                format!("router-assets-{alias_id}"),
                &host,
                &svc,
                &mut router,
            );
        }
        let key = format!("router-alias-{alias_id}");
        if ty == "environment" && value.as_deref() == Some("prod") {
            unprotected.insert(key.clone());
            unprotected.insert(format!("router-assets-{alias_id}"));
        }
        routers.insert(key, router);
    }

    for domain in &domains {
        // Domains route to the current deployment of their environment alias.
        let env_alias = aliases.iter().find(|(_, _, ty, value, _, _, _, _, _)| {
            ty == "environment_id" && value.as_deref() == domain.environment_id.as_deref()
        });
        let Some((_, deployment_id, _, _, _, rnode, rport, rhost, outdir)) = env_alias else {
            continue;
        };

        if domain.r#type == "route" {
            let svc = service_ref(deployment_id, rnode, *rport, rhost, &mut services);
            rum_hosts.push(domain.hostname.clone());
            let mut router = json!({
                "rule": format!("Host(`{}`)", domain.hostname),
                "service": svc,
                "entryPoints": entry_points,
            });
            if https {
                // lehttp: custom domains use the HTTP-01 resolver.
                router["tls"] = json!({ "certResolver": "lehttp" });
            }
            if is_static(outdir) {
                attach_cache(
                    &mut routers,
                    &mut middlewares,
                    format!("router-assets-dom-{}", domain.id),
                    &domain.hostname,
                    &svc,
                    &mut router,
                );
            }
            let key = format!("router-domain-{}", domain.id);
            if domain.environment_id.as_deref() == Some("prod") {
                unprotected.insert(key.clone());
                unprotected.insert(format!("router-assets-dom-{}", domain.id));
            }
            routers.insert(key, router);
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

    // Project firewall — `config.firewall` carries optional
    // `ip_allowlist: [cidr]` and `rate_limit: {average, burst}`; both
    // become Traefik middlewares attached to every deployment router.
    let fw = project.config.get("firewall");
    let mut fw_mws: Vec<String> = Vec::new();
    if let Some(allow) = fw
        .and_then(|f| f.get("ip_allowlist"))
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty())
    {
        middlewares.insert(
            "fw-allowlist".into(),
            json!({ "ipAllowList": { "sourceRange": allow } }),
        );
        fw_mws.push("fw-allowlist".into());
    }
    if let Some(rl) = fw.and_then(|f| f.get("rate_limit")) {
        let avg = rl.get("average").and_then(|v| v.as_u64()).unwrap_or(0);
        if avg > 0 {
            let burst = rl.get("burst").and_then(|v| v.as_u64()).unwrap_or(avg * 2);
            middlewares.insert(
                "fw-ratelimit".into(),
                json!({
                    "rateLimit": {
                        "average": avg,
                        "burst": burst,
                        "period": "1s",
                        "sourceCriterion": { "ipStrategy": { "depth": 2 } },
                    }
                }),
            );
            fw_mws.push("fw-ratelimit".into());
        }
    }
    if !fw_mws.is_empty() {
        for router in routers.values_mut() {
            if router["service"]
                .as_str()
                .is_some_and(|s| s.starts_with("deployment-"))
            {
                let existing: Vec<Value> = router["middlewares"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                // Firewall runs first — an attacker shouldn't hit the app
                // before the allowlist check.
                router["middlewares"] = json!(fw_mws
                    .iter()
                    .map(|m| json!(m))
                    .chain(existing)
                    .collect::<Vec<_>>());
            }
        }
    }

    // Deployment protection — `config.protection.users` (htpasswd-format,
    // bcrypt at PATCH time) gates every router except the prod
    // environment alias and prod-bound domains: Vercel's Deployment
    // Protection, self-hosted.
    if let Some(users) = project
        .config
        .get("protection")
        .and_then(|p| p.get("users"))
        .and_then(|u| u.as_array())
        .filter(|u| !u.is_empty())
    {
        middlewares.insert("protect".into(), json!({ "basicAuth": { "users": users } }));
        for (key, router) in routers.iter_mut() {
            if unprotected.contains(key) {
                continue;
            }
            if !router["service"]
                .as_str()
                .is_some_and(|s| s.starts_with("deployment-"))
            {
                continue;
            }
            // Order: firewall (deny early) → protect (auth) → cdn headers
            // — `immutable` must not reach a 401.
            let existing: Vec<String> = router["middlewares"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let (fw, rest): (Vec<String>, Vec<String>) =
                existing.into_iter().partition(|m| m.starts_with("fw-"));
            router["middlewares"] = json!(fw
                .into_iter()
                .chain(std::iter::once("protect".to_string()))
                .chain(rest)
                .collect::<Vec<_>>());
        }
    }

    // `/_runway-rum` → API. The longer rule wins over plain `Host()`.
    // `runway@docker` is the compose-defined service for the app itself.
    for host in rum_hosts {
        let mut router = json!({
            "rule": format!("Host(`{host}`) && PathPrefix(`/_runway-rum`)"),
            "service": "runway@docker",
            "entryPoints": entry_points,
        });
        if https {
            router["tls"] = json!({ "certResolver": "le" });
        }
        let key = format!("router-rum-{}", host.replace(['.', '_'], "-"));
        routers.insert(key, router);
    }

    // Traefik v3 rejects empty section maps in file-provider configs
    // ("services cannot be a standalone element"), which silently drops
    // every router in the file — omit sections with no entries.
    let mut http = serde_json::Map::new();
    if !routers.is_empty() {
        http.insert("routers".into(), Value::Object(routers));
    }
    if !middlewares.is_empty() {
        http.insert("middlewares".into(), Value::Object(middlewares));
    }
    if !services.is_empty() {
        http.insert("services".into(), Value::Object(services));
    }
    let doc = json!({ "http": http });
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
