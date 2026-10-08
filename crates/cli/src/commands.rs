//! User-facing CLI commands: login, link, deploy, logs, env, domains, open.
//! All talk to a running instance over the REST API via `client::Client`.

use std::io::Write;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::client::{self, CliConfig, Client, LinkFile};

/// `runway login` — store instance URL + API key after validating them.
pub async fn login(server: Option<String>, key: Option<String>) -> anyhow::Result<()> {
    let server = match server {
        Some(s) => s.trim_end_matches('/').to_string(),
        None => prompt("Instance URL (e.g. https://runway.example.com): ")?,
    };
    if !(server.starts_with("http://") || server.starts_with("https://")) {
        bail!("server must start with http:// or https://");
    }
    let key = match key {
        Some(k) => k,
        None => prompt("API key (ak_...): ")?,
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

/// `runway link [project-id]` — bind CWD to a project (`.runway/project.json`).
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
    let name = p["project"]["name"].as_str().map(String::from);
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
    let link = client::load_link()?;

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
        follow_deployment(&client, dep_id).await?;
    }
    Ok(())
}

/// `runway logs [deployment-id] [--follow]`.
pub async fn logs(deployment: Option<String>, follow: bool) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let dep_id = match deployment {
        Some(d) => d,
        None => latest_deployment(&client).await?,
    };
    let mut printed = print_logs(&client, &dep_id, 0).await?;
    if !follow {
        return Ok(());
    }
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        printed = print_logs(&client, &dep_id, printed).await?;
        // Terminal signal is `conclusion` — status='completed' covers
        // succeeded AND failed (devpush lifecycle semantics).
        let dep = client.get(&format!("/api/v1/deployments/{dep_id}")).await?;
        if let Some(conclusion) = dep["conclusion"].as_str() {
            let _ = print_logs(&client, &dep_id, printed).await?;
            println!("-- deployment concluded: {conclusion}");
            break;
        }
    }
    Ok(())
}

/// `runway deployments [--limit N]`.
pub async fn deployments(limit: usize) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let res = client
        .get(&format!(
            "/api/v1/projects/{}/deployments?limit={limit}",
            link.project_id
        ))
        .await?;
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

/// `runway env list | set KEY=VAL | unset KEY`.
pub async fn env(args: Vec<String>, environment: Option<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let base = format!("/api/v1/projects/{}/env", link.project_id);

    match args.first().map(String::as_str) {
        None | Some("list") => {
            let res = client.get(&base).await?;
            for v in res["env"].as_array().cloned().unwrap_or_default() {
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
            let key = args.get(1).context("usage: runway env unset KEY")?;
            client
                .patch(
                    &base,
                    &json!([{ "key": key, "environment": environment, "delete": true }]),
                )
                .await?;
            println!("Unset {key}");
        }
        Some(other) => bail!("unknown env action '{other}' — list|set|unset"),
    }
    Ok(())
}

/// `runway domains list | add <host> | remove <host> | assign-cf <host>`.
pub async fn domains(args: Vec<String>) -> anyhow::Result<()> {
    let client = Client::from_config()?;
    let link = client::load_link()?;
    let base = format!("/api/v1/projects/{}/domains", link.project_id);

    let find_id = |hostname: &str, list: &[Value]| -> anyhow::Result<i64> {
        list.iter()
            .find(|d| d["hostname"].as_str() == Some(hostname))
            .and_then(|d| d["id"].as_i64())
            .context("domain not found on project")
    };

    match args.first().map(String::as_str) {
        None | Some("list") => {
            let res = client.get(&base).await?;
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

// -- helpers ---------------------------------------------------------------

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

/// Tar+gzip CWD via the system `tar` (excludes VCS/build dirs). Returns bytes.
fn pack_cwd() -> anyhow::Result<Vec<u8>> {
    let out = std::process::Command::new("tar")
        .args([
            "-czf",
            "-",
            "--exclude=./.git",
            "--exclude=./node_modules",
            "--exclude=./.runway",
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

/// Print log lines after `skip`; returns the new line count.
async fn print_logs(client: &Client, dep_id: &str, skip: usize) -> anyhow::Result<usize> {
    let text = client
        .get_text(&format!("/api/v1/deployments/{dep_id}/logs?tail=5000"))
        .await?;
    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().skip(skip) {
        println!("{line}");
    }
    Ok(lines.len())
}

/// Poll a deployment until terminal status, printing transitions.
async fn follow_deployment(client: &Client, dep_id: &str) -> anyhow::Result<()> {
    let mut last = String::new();
    loop {
        let dep = client.get(&format!("/api/v1/deployments/{dep_id}")).await?;
        let status = dep["status"].as_str().unwrap_or("?").to_string();
        let computed = dep["computed_status"].as_str().unwrap_or("");
        let label = if computed.is_empty() || computed == status {
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
                let url = dep["urls"]["environment"]
                    .as_str()
                    .or(dep["urls"]["immutable"].as_str())
                    .unwrap_or("");
                println!("Live at https://{url}");
                return Ok(());
            }
            Some(other) => {
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
