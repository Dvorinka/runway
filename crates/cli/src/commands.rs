//! User-facing CLI commands: login, link, deploy, logs, env, domains, open.
//! All talk to a running instance over the REST API via `client::Client`.

use std::io::{IsTerminal, Write};

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::client::{self, CliConfig, Client, LinkFile};

/// `runway login` — store instance URL + API key after validating them.
pub async fn login(
    server: Option<String>,
    key: Option<String>,
    email: Option<String>,
    password: Option<String>,
) -> anyhow::Result<()> {
    let server = match server {
        Some(s) => s.trim_end_matches('/').to_string(),
        None => prompt("Instance URL (e.g. https://runway.example.com): ")?,
    };
    if !(server.starts_with("http://") || server.starts_with("https://")) {
        bail!("server must start with http:// or https://");
    }

    let key = match key {
        Some(k) => k,
        None => {
            // No key — email/password sign-in mints one, so a fresh
            // machine never needs the dashboard (or `bootstrap`).
            login_mint_key(&server, email, password).await?
        }
    };
    if !key.starts_with("ak_") {
        bail!("expected an ak_ API key (run `runway bootstrap` or the dashboard)");
    }

    // Validate before persisting.
    let client = Client::new(&server, &key);
    client
        .get("/api/v1/projects")
        .await
        .context("login failed — check server URL and key")?;

    client::save_config(&CliConfig {
        server: Some(server.clone()),
        key: Some(key),
    })?;
    println!(
        "Logged in to {server} ({})",
        client::config_path().display()
    );
    Ok(())
}

/// Sign in with email + password and mint an `ak_` API key. Handles the
/// TOTP challenge when the account has 2FA enabled.
async fn login_mint_key(
    server: &str,
    email: Option<String>,
    password: Option<String>,
) -> anyhow::Result<String> {
    let email = match email {
        Some(e) if !e.trim().is_empty() => e,
        _ => prompt("Email: ")?,
    };
    let password = match password {
        Some(p) if !p.is_empty() => p,
        _ => prompt_secret("Password: ")?,
    };
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{server}/api/auth/login"))
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .context("login request failed")?;
    let mut cookie = first_set_cookie(&resp);
    let body: Value = resp.json().await.unwrap_or_default();
    if body["two_factor"].as_bool() == Some(true) {
        let pending = body["pending"].as_str().unwrap_or_default().to_string();
        let code = prompt("Two-factor or recovery code: ")?;
        let resp = http
            .post(format!("{server}/api/auth/totp/challenge"))
            .json(&json!({ "pending": pending, "code": code }))
            .send()
            .await
            .context("2FA challenge failed")?;
        if !resp.status().is_success() {
            bail!("invalid two-factor code");
        }
        cookie = first_set_cookie(&resp).or(cookie);
    }
    let cookie = cookie.context("no session cookie returned — check email and password")?;

    // Session → named API key. Name ties the key to this machine so the
    // dashboard's key list stays auditable.
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "machine".into());
    let resp = http
        .post(format!("{server}/api/v1/keys"))
        .header("cookie", cookie)
        .json(&json!({ "name": format!("cli-{host}") }))
        .send()
        .await
        .context("key mint failed")?;
    if !resp.status().is_success() {
        bail!("could not mint API key ({})", resp.status());
    }
    let body: Value = resp.json().await.unwrap_or_default();
    body["token"]
        .as_str()
        .map(String::from)
        .context("no token in key response")
}

/// First `name=value` pair from a response's Set-Cookie header.
fn first_set_cookie(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next().map(str::to_string))
}

fn prompt_secret(msg: &str) -> anyhow::Result<String> {
    let s = rpassword::prompt_password(msg)?;
    if s.is_empty() {
        bail!("empty input");
    }
    Ok(s)
}

/// `runway link [project-id]` — bind CWD to a project (`.runway/project.json`).
/// `runway create [name] [--preset slug]` — create an upload-based
/// project from the current directory and link it (vercel-style first run).
pub async fn create(name: Option<String>, preset: Option<String>) -> anyhow::Result<String> {
    let client = Client::from_config()?;
    let name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => cwd_name()?,
    };
    let mut body = json!({
        "name": name,
        "provider": "upload",
        "repo_full_name": name,
    });
    if let Some(p) = preset {
        body["preset"] = json!(p);
    }
    let res = client.post("/api/v1/projects", &body).await?;
    let project_id = res["id"].as_str().context("no project id in response")?;
    let url = res["url"].as_str().unwrap_or("");
    client::save_link(&LinkFile {
        project_id: project_id.to_string(),
        project_name: Some(name.clone()),
    })?;
    println!("Created project {name} ({project_id}).");
    if !url.is_empty() {
        println!("URL: {url}");
    }
    println!("Linked this directory. Add `.runway/` to .gitignore if unwanted.");
    Ok(project_id.to_string())
}

fn cwd_name() -> anyhow::Result<String> {
    let cwd = std::env::current_dir()?;
    cwd.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .context("cannot determine project name — pass one explicitly")
}

/// `runway whoami` — show the logged-in user and instance health.
pub async fn whoami() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let me = client.get("/api/auth/me").await?;
    let email = me["email"].as_str().unwrap_or("?");
    let username = me["username"].as_str().unwrap_or("");
    let health = client
        .get("/health")
        .await
        .ok()
        .and_then(|h| h["status"].as_str().map(String::from))
        .unwrap_or_else(|| "unreachable".to_string());
    let server = client::load_config().server.unwrap_or_default();
    if username.is_empty() {
        println!("{email}");
    } else {
        println!("{email} ({username})");
    }
    println!("server: {server} [{health}]");
    Ok(())
}

/// `runway logout` — delete stored credentials on this machine.
pub async fn logout() -> anyhow::Result<()> {
    let path = client::config_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Logged out ({})", path.display());
    } else {
        println!("Not logged in.");
    }
    Ok(())
}

/// `runway unlink` — forget the linked project for this directory.
pub async fn unlink() -> anyhow::Result<()> {
    let path = client::link_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Unlinked this directory.");
    } else {
        println!("Directory is not linked.");
    }
    Ok(())
}

/// `runway projects` — list all projects on the instance.
pub async fn projects(json_out: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let res = client.get("/api/v1/projects").await?;
    if json_out {
        return print_json(&res);
    }
    let list = res["projects"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("No projects yet — run `runway create <name>`.");
        return Ok(());
    }
    for p in &list {
        let url = p["url"].as_str().unwrap_or("");
        println!(
            "{}  {}{}",
            p["id"].as_str().unwrap_or("?"),
            p["name"].as_str().unwrap_or("?"),
            if url.is_empty() {
                String::new()
            } else {
                // Project URLs already carry the scheme.
                format!("  {url}")
            }
        );
    }
    Ok(())
}

/// `runway status` — instance, link, project, and latest deployment state.
pub async fn status() -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let server = client::load_config().server.unwrap_or_default();
    let health = client
        .get("/health")
        .await
        .ok()
        .and_then(|h| h["status"].as_str().map(String::from))
        .unwrap_or_else(|| "unreachable".to_string());
    println!("server: {server} [{health}]");
    let Ok(link) = client::load_link() else {
        println!("directory: not linked (run `runway link` or `runway create`)");
        return Ok(());
    };
    let p = client
        .get(&format!("/api/v1/projects/{}", link.project_id))
        .await?;
    println!(
        "project: {} ({})",
        p["name"].as_str().unwrap_or("?"),
        link.project_id
    );
    if let Some(url) = p["url"].as_str().filter(|u| !u.is_empty()) {
        println!("url: {url}");
    }
    let deps = client
        .get(&format!(
            "/api/v1/projects/{}/deployments?limit=1",
            link.project_id
        ))
        .await?;
    if let Some(d) = deps["deployments"].as_array().and_then(|a| a.first()) {
        let conclusion = d["conclusion"].as_str().unwrap_or("");
        let state = if conclusion.is_empty() {
            d["status"].as_str().unwrap_or("?").to_string()
        } else {
            format!("{} ({conclusion})", d["status"].as_str().unwrap_or("?"))
        };
        println!(
            "latest: {} {state} {}",
            d["id"].as_str().unwrap_or("?"),
            d["created_at"].as_str().unwrap_or("")
        );
    } else {
        println!("latest: no deployments yet");
    }
    Ok(())
}

/// `runway redeploy [id]` — new deployment from the same source.
pub async fn redeploy(deployment: Option<String>, follow: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let id = match deployment {
        Some(id) => id,
        None => latest_deployment(&client).await?,
    };
    let res = client
        .post(&format!("/api/v1/deployments/{id}/redeploy"), &json!({}))
        .await?;
    let new_id = res["id"].as_str().context("no deployment id in response")?;
    println!("Redeployed as {new_id}");
    if follow {
        follow_deployment(&client, new_id).await?;
    }
    Ok(())
}

pub async fn link(project: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let project_id = match project {
        Some(p) => p,
        None => {
            let res = client.get("/api/v1/projects").await?;
            let projects = res["projects"].as_array().cloned().unwrap_or_default();
            for p in &projects {
                println!(
                    "  {}  {}",
                    p["id"].as_str().unwrap_or("?"),
                    p["name"].as_str().unwrap_or("?")
                );
            }
            prompt("Project ID to link: ")?
        }
    };
    let p = client
        .get(&format!("/api/v1/projects/{project_id}"))
        .await?;
    let name = p["name"].as_str().map(String::from);
    client::save_link(&LinkFile {
        project_id: project_id.clone(),
        project_name: name.clone(),
    })?;
    println!(
        "Linked to {} ({project_id}). Add `.runway/` to .gitignore if unwanted.",
        name.as_deref().unwrap_or("project")
    );
    Ok(())
}

/// `runway deploy` — tar the current directory and upload-deploy it.
pub async fn deploy(follow: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = match client::load_link() {
        Ok(l) => l,
        Err(e) if std::io::stdin().is_terminal() => {
            println!("This directory is not linked to a project yet.");
            if !confirm_yes("Create a new upload-based project from this directory?")? {
                bail!("{e} — or run `runway create <name>` first");
            }
            let def = cwd_name().unwrap_or_else(|_| "site".to_string());
            let name = prompt_default(&format!("Project name [{def}]:"), &def);
            let preset = prompt_default("Preset (static, nextjs, … — empty = auto-detect):", "");
            create(
                if name.is_empty() { None } else { Some(name) },
                if preset.is_empty() {
                    None
                } else {
                    Some(preset)
                },
            )
            .await?;
            client::load_link()?
        }
        Err(_) if !std::io::stdin().is_terminal() => {
            bail!("no linked project — run `runway link <project-id>` or `runway create <name>`");
        }
        Err(e) => bail!("{e} — run `runway create <name>` first"),
    };

    let tarball = pack_cwd()?;
    println!(
        "Uploading {} bytes to project {} ...",
        tarball.len(),
        link.project_id
    );
    let dep = client
        .upload(
            &format!("/api/v1/projects/{}/deployments/upload", link.project_id),
            tarball,
        )
        .await?;
    let dep_id = dep["id"].as_str().context("no deployment id in response")?;
    let url = dep["urls"]["immutable"].as_str().unwrap_or("");
    println!("Deployment {dep_id} queued — https://{url}");

    if follow {
        let started = std::time::Instant::now();
        follow_deployment(&client, dep_id).await?;
        println!("done in {}s", started.elapsed().as_secs());
    } else {
        println!("Stream logs with `runway logs --follow` (or re-run with --follow).");
    }
    Ok(())
}

/// `runway logs [deployment-id] [--follow] [--project]`.
pub async fn logs(deployment: Option<String>, follow: bool, project: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    if project {
        let link = client::load_link()?;
        let text = client
            .get_text(&format!("/api/v1/projects/{}/logs", link.project_id))
            .await?;
        for line in text.lines() {
            print_log_line(line, false);
        }
        return Ok(());
    }
    let dep_id = match deployment {
        Some(d) => d,
        None => latest_deployment(&client).await?,
    };
    let mut printed = print_logs(&client, &dep_id, 0, false).await?;
    if !follow {
        return Ok(());
    }
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        printed = print_logs(&client, &dep_id, printed, false).await?;
        // Terminal signal is `conclusion` — status='completed' covers
        // succeeded AND failed (devpush lifecycle semantics).
        let dep = client.get(&format!("/api/v1/deployments/{dep_id}")).await?;
        if let Some(conclusion) = dep["conclusion"].as_str() {
            let _ = print_logs(&client, &dep_id, printed, false).await?;
            println!("-- deployment concluded: {conclusion}");
            break;
        }
    }
    Ok(())
}

/// `runway deployments [--limit N]`.
pub async fn deployments(limit: usize, json_out: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let res = client
        .get(&format!(
            "/api/v1/projects/{}/deployments?limit={limit}",
            link.project_id
        ))
        .await?;
    if json_out {
        return print_json(&res);
    }
    let deps = res["deployments"].as_array().cloned().unwrap_or_default();
    if deps.is_empty() {
        println!("No deployments yet.");
        return Ok(());
    }
    for d in deps {
        let sha = d["commit_sha"].as_str().unwrap_or("");
        let short = &sha[..sha.len().min(7)];
        let status = d["conclusion"]
            .as_str()
            .or_else(|| d["status"].as_str())
            .unwrap_or("-");
        let created = d["created_at"].as_str().unwrap_or("");
        let msg = d["commit_meta"]["message"].as_str().unwrap_or("");
        println!("{}  {:<10}  {:<19}  {}", short, status, created, msg);
    }
    Ok(())
}

/// `runway stats [deployment]` — live container resource snapshot.
pub async fn stats(deployment: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let dep_id = match deployment {
        Some(d) => d,
        None => latest_deployment(&client).await?,
    };
    let s = client
        .get(&format!("/api/v1/deployments/{dep_id}/stats"))
        .await?;
    if !s["running"].as_bool().unwrap_or(false) {
        println!("{} — container not running", &dep_id[..dep_id.len().min(7)]);
        return Ok(());
    }
    let fmt_b = |v: &serde_json::Value| {
        let n = v.as_u64().unwrap_or(0) as f64;
        if n >= (1u64 << 30) as f64 {
            format!("{:.1} GB", n / (1u64 << 30) as f64)
        } else if n >= (1u64 << 20) as f64 {
            format!("{:.0} MB", n / (1u64 << 20) as f64)
        } else {
            format!("{:.0} KB", n / (1u64 << 10) as f64)
        }
    };
    println!(
        "{}  cpu {:>5.1}%  mem {} / {}  net ↓{} ↑{}  pids {}",
        &dep_id[..dep_id.len().min(7)],
        s["cpu_pct"].as_f64().unwrap_or(0.0),
        fmt_b(&s["mem_used"]),
        fmt_b(&s["mem_limit"]),
        fmt_b(&s["net_rx"]),
        fmt_b(&s["net_tx"]),
        s["pids"].as_u64().unwrap_or(0),
    );
    Ok(())
}

/// `runway rollback [environment]`.
pub async fn rollback(environment: String) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let res = client
        .post(
            &format!(
                "/api/v1/projects/{}/environments/{environment}/rollback",
                link.project_id
            ),
            &serde_json::json!({}),
        )
        .await?;
    let dep = res["deployment_id"].as_str().unwrap_or("?");
    println!("{environment} rolled back to deployment {dep}");
    Ok(())
}

/// `runway promote [deployment] [--environment prod]` — point an
/// environment's alias at a deployment (Vercel's "Promote to production").
pub async fn promote(deployment: Option<String>, environment: String) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let id = match deployment {
        Some(id) => id,
        None => latest_deployment(&client).await?,
    };
    let res = client
        .post(
            &format!("/api/v1/deployments/{id}/promote"),
            &json!({ "environment_id": &environment }),
        )
        .await?;
    let env = res["environment_id"].as_str().unwrap_or(&environment);
    let dep = res["deployment_id"].as_str().unwrap_or(&id);
    println!("{env} now serves deployment {dep}");
    Ok(())
}

/// `runway cancel [deployment]` — cancel a running deployment.
pub async fn cancel(deployment: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let id = match deployment {
        Some(id) => id,
        None => latest_deployment(&client).await?,
    };
    let res = client
        .post(&format!("/api/v1/deployments/{id}/cancel"), &json!({}))
        .await?;
    let status = res["status"].as_str().unwrap_or("canceled");
    println!("Deployment {id} {status}");
    Ok(())
}

/// `runway skip [deployment]` — drop a queued deployment before it runs.
pub async fn skip(deployment: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let id = match deployment {
        Some(id) => id,
        None => latest_deployment(&client).await?,
    };
    client
        .post(&format!("/api/v1/deployments/{id}/skip"), &json!({}))
        .await?;
    println!("Deployment {id} skipped");
    Ok(())
}

/// `runway env list | set KEY=VAL | unset KEY`.
pub async fn env(mut args: Vec<String>, environment: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let base = format!("/api/v1/projects/{}/env", link.project_id);
    let json_out = strip_json_flag(&mut args);

    match args.first().map(String::as_str) {
        None | Some("list") => {
            let res = client.get(&base).await?;
            if json_out {
                return print_json(&res);
            }
            let vars = res["env"].as_array().cloned().unwrap_or_default();
            if vars.is_empty() {
                println!("No variables — `runway env set KEY=VALUE` or `runway env push [file]`.");
                return Ok(());
            }
            for v in &vars {
                let env_tag = v["environment"]
                    .as_str()
                    .map(|e| format!(" [{e}]"))
                    .unwrap_or_default();
                println!(
                    "{}={}{}",
                    v["key"].as_str().unwrap_or(""),
                    v["value"].as_str().unwrap_or(""),
                    env_tag
                );
            }
        }
        Some("set") => {
            let spec = args.get(1).context("usage: runway env set KEY=VALUE")?;
            let (key, value) = spec
                .split_once('=')
                .context("usage: runway env set KEY=VALUE")?;
            client
                .patch(
                    &base,
                    &json!([{ "key": key, "value": value, "environment": environment }]),
                )
                .await?;
            println!("Set {key}");
        }
        Some("unset") | Some("rm") => {
            let keys: Vec<&String> = args.iter().skip(1).collect();
            if keys.is_empty() {
                bail!("usage: runway env unset KEY [KEY...]");
            }
            for key in keys {
                client
                    .patch(
                        &base,
                        &json!([{ "key": key, "environment": environment, "delete": true }]),
                    )
                    .await?;
                println!("Unset {key}");
            }
        }
        // `runway env pull [path] [--force]` — dotenv via the project
        // export (the only read that decrypts; writer-gated like the
        // UI's export button). Defaults to `.env.local`, `-` for stdout.
        Some("pull") => {
            let path = match args.get(1).map(String::as_str) {
                Some("-") => "-",
                Some(a) if !a.starts_with('-') => a,
                _ => ".env.local",
            };
            let force = args.iter().any(|a| a == "--force" || a == "-f");
            let res = client
                .get(&format!("/api/v1/projects/{}/export", link.project_id))
                .await?;
            let mut out = String::new();
            for v in res["environment_variables"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let var_env = v["environment"].as_str();
                if environment.is_some() && var_env.is_some() && var_env != environment.as_deref() {
                    continue;
                }
                let (key, value) = (
                    v["key"].as_str().unwrap_or(""),
                    v["value"].as_str().unwrap_or(""),
                );
                if value
                    .chars()
                    .any(|c| c.is_whitespace() || c == '"' || c == '#')
                {
                    out.push_str(&format!("{key}=\"{}\"\n", value.replace('"', "\\\"")));
                } else {
                    out.push_str(&format!("{key}={value}\n"));
                }
            }
            if path == "-" {
                print!("{out}");
                return Ok(());
            }
            let file = std::path::Path::new(path);
            if file.exists() && !force {
                bail!("{path} exists — pass --force to overwrite");
            }
            tokio::fs::write(file, &out).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
            }
            println!("Wrote {path}");
        }
        // `runway env push [file]` — read a dotenv file and set each entry
        // (blank lines, `#` comments, and an optional `export ` prefix are
        // skipped; surrounding quotes are stripped).
        Some("push") => {
            let path = match args.get(1).map(String::as_str) {
                Some(a) if !a.starts_with('-') => a,
                _ => ".env.local",
            };
            let raw = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("cannot read {path}"))?;
            let mut vars = vec![];
            for (n, line) in raw.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let line = line.strip_prefix("export ").unwrap_or(line).trim();
                let (key, value) = line
                    .split_once('=')
                    .with_context(|| format!("{path}:{}: expected KEY=VALUE", n + 1))?;
                let key = key.trim().to_string();
                let mut value = value.trim().to_string();
                if value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
                    || value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'')
                {
                    value = value[1..value.len() - 1].to_string();
                }
                anyhow::ensure!(!key.is_empty(), "{path}:{}: empty key", n + 1);
                vars.push(json!({ "key": key, "value": value, "environment": environment }));
            }
            if vars.is_empty() {
                println!("Nothing to push from {path}.");
                return Ok(());
            }
            client.patch(&base, &Value::Array(vars.clone())).await?;
            println!(
                "Pushed {} variable{} from {path}",
                vars.len(),
                if vars.len() == 1 { "" } else { "s" }
            );
        }
        Some(other) => bail!("unknown env action '{other}' — list|set|unset|pull|push"),
    }
    Ok(())
}

/// `runway domains list | add <host> | remove <host> | assign-cf <host>`.
pub async fn domains(mut args: Vec<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let base = format!("/api/v1/projects/{}/domains", link.project_id);
    let json_out = strip_json_flag(&mut args);

    let find_id = |hostname: &str, list: &[Value]| -> anyhow::Result<i64> {
        list.iter()
            .find(|d| d["hostname"].as_str() == Some(hostname))
            .and_then(|d| d["id"].as_i64())
            .context("domain not found on project")
    };

    match args.first().map(String::as_str) {
        None | Some("list") => {
            let res = client.get(&base).await?;
            if json_out {
                return print_json(&res);
            }
            for d in res["domains"].as_array().cloned().unwrap_or_default() {
                println!(
                    "  {:<40} {} [{}]",
                    d["hostname"].as_str().unwrap_or(""),
                    d["type"].as_str().unwrap_or("route"),
                    if d["cloudflare_record_id"].is_null() {
                        "manual"
                    } else {
                        "cloudflare"
                    },
                );
            }
        }
        Some("add") => {
            let host = args
                .get(1)
                .context("usage: runway domains add <hostname>")?;
            client.post(&base, &json!({ "hostname": host })).await?;
            println!("Added {host}");
        }
        Some("remove") | Some("rm") => {
            let host = args
                .get(1)
                .context("usage: runway domains remove <hostname>")?;
            let res = client.get(&base).await?;
            let id = find_id(
                host,
                res["domains"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            )?;
            client.delete(&format!("{base}/{id}")).await?;
            println!("Removed {host}");
        }
        Some("assign-cf") | Some("cloudflare") => {
            let host = args
                .get(1)
                .context("usage: runway domains assign-cf <hostname>")?;
            let res = client.get(&base).await?;
            let id = find_id(
                host,
                res["domains"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            )?;
            client
                .post(&format!("{base}/{id}/assign-cloudflare"), &json!({}))
                .await?;
            println!("Assigned {host} via Cloudflare");
        }
        Some(other) => bail!("unknown domains action '{other}' — list|add|remove|assign-cf"),
    }
    Ok(())
}

/// `runway open` — print (and best-effort open) the project's production URL.
pub async fn open(print_only: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let dep_id = latest_deployment(&client).await?;
    let dep = client.get(&format!("/api/v1/deployments/{dep_id}")).await?;
    let host = dep["urls"]["environment"]
        .as_str()
        .or(dep["urls"]["immutable"].as_str())
        .context("deployment has no URL yet")?;
    let url = format!("https://{host}");
    println!("{url}");
    if !print_only {
        open_browser(&url);
    }
    Ok(())
}

async fn list_nodes(client: &Client) -> anyhow::Result<Vec<Value>> {
    client
        .get("/api/v1/admin/nodes")
        .await
        .map(|r| r["nodes"].as_array().cloned().unwrap_or_default())
}

async fn find_node(client: &Client, want: &str) -> anyhow::Result<Value> {
    list_nodes(client)
        .await?
        .into_iter()
        .find(|n| n["id"].as_str() == Some(want) || n["name"].as_str() == Some(want))
        .context("no node with that id or name")
}

/// `runway nodes` — admin surface for remote Docker nodes.
///   nodes [ls]                          list nodes with status + inventory
///   nodes add --command [--expires H]   print the one-line install command
///   nodes approve <id|name>             approve a waiting node
///   nodes rm <id|name>                  delete/reject a node
///   nodes token ls | token rm <id>      list or revoke enrollment tokens
pub async fn nodes(mut args: Vec<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let base = "/api/v1/admin/nodes";
    let json_out = strip_json_flag(&mut args);

    match args.first().map(String::as_str) {
        None | Some("ls") | Some("list") => {
            if json_out {
                let res = client.get(base).await?;
                return print_json(&res);
            }
            for n in list_nodes(&client).await? {
                let meta = &n["meta"];
                let inv = [
                    meta["arch"].as_str().map(str::to_string),
                    meta["docker_version"]
                        .as_str()
                        .map(|v| format!("docker {v}")),
                    meta["cpus"].as_u64().map(|c| format!("{c} cpu")),
                    meta["mem_bytes"]
                        .as_u64()
                        .map(|m| format!("{:.1} GB", m as f64 / 1e9)),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                println!(
                    "  {:<16} {:<8} {:<32} {} · seen {}",
                    n["name"].as_str().unwrap_or(""),
                    n["status"].as_str().unwrap_or("?"),
                    n["docker_url"].as_str().unwrap_or(""),
                    inv,
                    n["updated_at"].as_str().map(ago).unwrap_or_default(),
                );
            }
        }
        Some("add") => {
            // nodes add --command [--expires <hours>]
            anyhow::ensure!(
                args.iter().any(|a| a == "--command"),
                "usage: runway nodes add --command [--expires <hours>]"
            );
            let hours: i64 = args
                .iter()
                .position(|a| a == "--expires")
                .and_then(|i| args.get(i + 1))
                .map(|v| v.parse().context("--expires must be hours"))
                .transpose()?
                .unwrap_or(24);
            let res = client
                .post(
                    &format!("{base}/tokens"),
                    &json!({ "expires_hours": hours }),
                )
                .await?;
            let token = res["token"].as_str().context("no token in response")?;
            let server = client.base_url();
            println!("Run this on the machine to enroll it as a deploy node:\n");
            println!(
                "  curl -fsSL {server}/install-node.sh | sudo RUNWAY_URL={server} RUNWAY_TOKEN={token} sh\n"
            );
            println!("Token expires in {hours}h. The node lands as 'waiting' — approve it with:");
            println!("  runway nodes approve <name>");
        }
        Some("approve") => {
            let want = args
                .get(1)
                .context("usage: runway nodes approve <id|name>")?;
            let n = find_node(&client, want).await?;
            let id = n["id"].as_str().unwrap();
            let res = client
                .post(&format!("{base}/{id}/approve"), &json!({}))
                .await?;
            println!(
                "{} → {}",
                res["name"].as_str().unwrap_or(want),
                res["status"].as_str().unwrap_or("?")
            );
        }
        Some("rm") | Some("remove") | Some("delete") => {
            let want = args.get(1).context("usage: runway nodes rm <id|name>")?;
            let n = find_node(&client, want).await?;
            client
                .delete(&format!("{base}/{}", n["id"].as_str().unwrap()))
                .await?;
            println!("Removed {}", n["name"].as_str().unwrap_or(want));
        }
        Some("token") => match args.get(1).map(String::as_str) {
            Some("ls") | Some("list") => {
                let res = client.get(&format!("{base}/tokens")).await?;
                if json_out {
                    return print_json(&res);
                }
                for t in res["tokens"].as_array().cloned().unwrap_or_default() {
                    let state = if t["revoked"].as_bool() == Some(true) {
                        "revoked"
                    } else if t["expired"].as_bool() == Some(true) {
                        "expired"
                    } else {
                        "active"
                    };
                    println!(
                        "  {:<34} {:<8} expires {}",
                        t["id"].as_str().unwrap_or(""),
                        state,
                        t["expires_at"].as_str().unwrap_or(""),
                    );
                }
            }
            Some("rm") | Some("revoke") => {
                let id = args.get(2).context("usage: runway nodes token rm <id>")?;
                client.delete(&format!("{base}/tokens/{id}")).await?;
                println!("Revoked {id}");
            }
            _ => bail!("usage: runway nodes token ls | token rm <id>"),
        },
        Some(other) => bail!(
            "unknown nodes action '{other}' — ls | add --command | approve | rm | token ls|rm"
        ),
    }
    Ok(())
}

// -- helpers ---------------------------------------------------------------

/// Pull `--json` out of trailing-var-arg command args; returns whether
/// machine-readable output was requested.
fn strip_json_flag(args: &mut Vec<String>) -> bool {
    let had = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    had
}

/// RFC3339 → "5m ago" / "2h ago" / "3d ago"; falls back to the date prefix.
fn ago(iso: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return iso.get(..10).unwrap_or(iso).to_string();
    };
    let s = chrono::Utc::now()
        .signed_duration_since(t)
        .num_seconds()
        .max(0);
    match s {
        0..=89 => "just now".into(),
        90..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn print_json(v: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn prompt(msg: &str) -> anyhow::Result<String> {
    print!("{msg}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    let s = s.trim().to_string();
    if s.is_empty() {
        bail!("empty input");
    }
    Ok(s)
}

/// Prompt with a default — empty input returns `default`.
fn prompt_default(msg: &str, default: &str) -> String {
    print!("{msg} ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s).is_err() {
        return default.to_string();
    }
    let s = s.trim().to_string();
    if s.is_empty() {
        default.to_string()
    } else {
        s
    }
}

/// Yes/no prompt, defaulting to yes.
fn confirm_yes(msg: &str) -> anyhow::Result<bool> {
    let s = prompt_default(&format!("{msg} [Y/n]:"), "y");
    Ok(!matches!(s.to_lowercase().as_str(), "n" | "no"))
}

/// Tar+gzip CWD via the system `tar` (excludes VCS/build dirs). Returns bytes.
fn pack_cwd() -> anyhow::Result<Vec<u8>> {
    let out = std::process::Command::new("tar")
        .args([
            "-czf",
            "-",
            "--exclude=./.git",
            "--exclude=./node_modules",
            "--exclude=./.runway",
            "--exclude=./.vercel",
            "--exclude=./target",
            "--exclude=./.next",
            "--exclude=./.turbo",
            ".",
        ])
        .output()
        .context("failed to run `tar` — required for `runway deploy`")?;
    if !out.status.success() {
        bail!("tar failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout)
}

async fn latest_deployment(client: &Client) -> anyhow::Result<String> {
    let link = client::load_link()?;
    let res = client
        .get(&format!(
            "/api/v1/projects/{}/deployments?limit=1",
            link.project_id
        ))
        .await?;
    res["deployments"]
        .as_array()
        .and_then(|d| d.first())
        .and_then(|d| d["id"].as_str())
        .map(String::from)
        .context("no deployments for linked project")
}

/// Print one stored log line; `step` markers render as section headers
/// instead of the raw `<ts>\tstep\t<name>` triple.
fn print_log_line(line: &str, strip_ts: bool) {
    let mut parts = line.splitn(3, '\t');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_), Some("step"), Some(name)) => println!("--- {name} ---"),
        (Some(_), Some(_), Some(text)) if strip_ts => println!("{text}"),
        _ => println!("{line}"),
    }
}

/// Print log lines after `skip`; returns the new line count. `strip_ts`
/// drops the timestamp column — nicer while a deploy streams.
async fn print_logs(
    client: &Client,
    dep_id: &str,
    skip: usize,
    strip_ts: bool,
) -> anyhow::Result<usize> {
    let text = client
        .get_text(&format!("/api/v1/deployments/{dep_id}/logs?tail=5000"))
        .await?;
    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().skip(skip) {
        print_log_line(line, strip_ts);
    }
    Ok(lines.len())
}

/// Poll a deployment until terminal status, streaming new log lines and
/// printing transitions (Vercel-style `deploy` output).
async fn follow_deployment(client: &Client, dep_id: &str) -> anyhow::Result<()> {
    let mut last = String::new();
    let mut printed = 0usize;
    loop {
        printed = print_logs(client, dep_id, printed, true)
            .await
            .unwrap_or(printed);
        let dep = client.get(&format!("/api/v1/deployments/{dep_id}")).await?;
        let status = dep["status"].as_str().unwrap_or("?").to_string();
        let computed = dep["computed_status"].as_str().unwrap_or("");
        let label = if computed.is_empty() || computed == "unknown" || computed == status {
            status.clone()
        } else {
            format!("{status} ({computed})")
        };
        if label != last {
            println!("status: {label}");
            last = label;
        }
        match dep["conclusion"].as_str() {
            Some("succeeded") => {
                let env = dep["urls"]["environment"].as_str().unwrap_or("");
                let imm = dep["urls"]["immutable"].as_str().unwrap_or("");
                if !env.is_empty() && env != imm {
                    println!("Live at https://{env} (immutable: https://{imm})");
                } else {
                    println!("Live at https://{imm}");
                }
                return Ok(());
            }
            Some(other) => {
                let _ = print_logs(client, dep_id, printed, true).await;
                let err = dep["error"]["message"]
                    .as_str()
                    .or(dep["error"].as_str())
                    .unwrap_or("no detail");
                bail!("deployment {other}: {err}");
            }
            None => {}
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

fn open_browser(url: &str) {
    // Best-effort; absence of a browser is not an error.
    for cmd in ["xdg-open", "open"] {
        if std::process::Command::new(cmd)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return;
        }
    }
}
