//! GitHub integration routes: webhook receiver, app-level repo browsing
//! for project creation, and the app-manifest registration flow.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::deploy::{self, CommitInfo};
use runway_core::github::GithubService;
use runway_core::models::Project;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Webhook — HMAC-verified, handles installation/repo/push events.
// Port of routers/github.py:github_webhook.
// ---------------------------------------------------------------------------

pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let Some(secret) = state.github.webhook_secret() else {
        return Err(ApiError::bad_request("webhook secret not configured"));
    };
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !GithubService::verify_webhook(&secret, &body, signature) {
        return Err(ApiError::unauthorized("invalid signature"));
    }
    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid json"))?;

    match event.as_str() {
        "installation" => {
            let installation_id = data["installation"]["id"].as_i64().unwrap_or(0);
            match data["action"].as_str().unwrap_or("") {
                "deleted" | "suspended" | "unsuspended" => {
                    let status = data["action"].as_str().unwrap();
                    sqlx::query(
                        "UPDATE github_installation SET status = $1 WHERE installation_id = $2",
                    )
                    .bind(if status == "unsuspended" {
                        "active"
                    } else {
                        status
                    })
                    .bind(installation_id)
                    .execute(&state.db)
                    .await?;
                }
                "created" if state.github.configured() => {
                    if let Ok((token, expires)) = state
                        .github
                        .installation_access_token(installation_id)
                        .await
                    {
                        let enc = state.crypto.encrypt(&token)?;
                        sqlx::query(
                            "INSERT INTO github_installation
                             (installation_id, token, token_expires_at, status)
                             VALUES ($1,$2,$3,'active')
                             ON CONFLICT (installation_id) DO UPDATE
                             SET token = $2, token_expires_at = $3, status = 'active'",
                        )
                        .bind(installation_id)
                        .bind(&enc)
                        .bind(expires)
                        .execute(&state.db)
                        .await?;
                    }
                }
                _ => {}
            }
        }
        "installation_repositories" => {
            let (key, repo_status) = match data["action"].as_str().unwrap_or("") {
                "removed" => ("repositories_removed", "removed"),
                "added" => ("repositories_added", "active"),
                _ => ("", ""),
            };
            if !key.is_empty() {
                let ids: Vec<i64> = data[key]
                    .as_array()
                    .map(|a| a.iter().filter_map(|r| r["id"].as_i64()).collect())
                    .unwrap_or_default();
                if !ids.is_empty() {
                    sqlx::query(
                        "UPDATE project SET repo_status = $1
                         WHERE repo_id = ANY($2) AND repo_provider = 'github'",
                    )
                    .bind(repo_status)
                    .bind(&ids)
                    .execute(&state.db)
                    .await?;
                }
            }
        }
        "repository" => {
            let repo_id = data["repository"]["id"].as_i64().unwrap_or(0);
            match data["action"].as_str().unwrap_or("") {
                "deleted" | "transferred" => {
                    sqlx::query(
                        "UPDATE project SET repo_status = $1
                         WHERE repo_id = $2 AND repo_provider = 'github'",
                    )
                    .bind(data["action"].as_str().unwrap())
                    .bind(repo_id)
                    .execute(&state.db)
                    .await?;
                }
                "renamed" => {
                    sqlx::query(
                        "UPDATE project SET repo_full_name = $1
                         WHERE repo_id = $2 AND repo_provider = 'github'",
                    )
                    .bind(data["repository"]["full_name"].as_str().unwrap_or(""))
                    .bind(repo_id)
                    .execute(&state.db)
                    .await?;
                }
                _ => {}
            }
        }
        "push" => {
            handle_push(&state, &data).await;
        }
        "pull_request" => {
            handle_pull_request(&state, &data).await;
        }
        "check_run" => {
            // "Re-run" on a check we created — external_id carries the
            // deployment id; redeploy from the same source.
            if data["action"].as_str() == Some("rerequested") {
                if let Some(dep_id) = data["check_run"]["external_id"].as_str() {
                    rerun_deployment(&state, dep_id).await;
                }
            }
        }
        "issue_comment" => {
            // `@runway deploy` on a PR redeploys its latest preview.
            if data["action"].as_str() == Some("created")
                && data["issue"]["pull_request"].is_object()
            {
                let body = data["comment"]["body"].as_str().unwrap_or("");
                let pr = data["issue"]["number"].as_i64().unwrap_or(0);
                if pr > 0 && is_deploy_command(body) {
                    rerun_pr_preview(&state, &data, pr).await;
                }
            }
        }
        _ => {}
    }
    Ok(StatusCode::OK.into_response())
}

/// push → for each active project on the repo: apply deployment rules,
/// create deployment, enqueue job. Port of the `push` webhook case.
async fn handle_push(state: &AppState, data: &Value) {
    let repo_id = data["repository"]["id"].as_i64().unwrap_or(0);
    let projects: Vec<Project> = match sqlx::query_as(
        "SELECT * FROM project
         WHERE repo_id = $1 AND repo_provider = 'github' AND status = 'active'",
    )
    .bind(repo_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "push webhook: project lookup failed");
            return;
        }
    };
    if projects.is_empty() {
        return;
    }

    let branch = data["ref"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("refs/heads/")
        .to_string();
    let commit = CommitInfo {
        sha: data["after"].as_str().unwrap_or("").to_string(),
        author: data["pusher"]["name"].as_str().unwrap_or("").to_string(),
        message: data["head_commit"]["message"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        timestamp: data["head_commit"]["timestamp"].as_str().map(String::from),
    };

    let changed = crate::routes::git_providers::changed_paths(data);

    for project in projects {
        if !crate::routes::git_providers::rules_allow(&project, &branch, &commit, &changed) {
            continue;
        }

        match deploy::create(
            &state.db,
            &state.bus,
            &state.crypto,
            &project,
            &branch,
            &commit,
            "webhook",
            None,
            None,
        )
        .await
        {
            Ok(dep) => tracing::info!(
                deployment_id = dep.id,
                project_id = project.id,
                "deployment created from push"
            ),
            Err(e) => {
                tracing::warn!(project_id = project.id, error = %e, "push: deployment create failed")
            }
        }
    }
}

/// pull_request opened/synchronize/reopened → preview deployment on the
/// PR head branch; closed → teardown the preview's aliases + containers.
/// Branch allowlists don't apply — previews are the point.
/// `auto_deploy=false` still opts the project out.
async fn handle_pull_request(state: &AppState, data: &Value) {
    let action = data["action"].as_str().unwrap_or("");
    if !matches!(
        action,
        "opened"
            | "synchronize"
            | "reopened"
            | "closed"
            | "labeled"
            | "unlabeled"
            | "ready_for_review"
            | "converted_to_draft"
    ) {
        return;
    }
    let repo_id = data["repository"]["id"].as_i64().unwrap_or(0);
    let pr = &data["pull_request"];
    let branch = pr["head"]["ref"].as_str().unwrap_or("").to_string();
    let sha = pr["head"]["sha"].as_str().unwrap_or("").to_string();
    if branch.is_empty() || sha.is_empty() {
        return;
    }
    let number = pr["number"].as_i64().unwrap_or(0);
    let is_draft = pr["draft"].as_bool().unwrap_or(false);
    // Head repo id differs from base repo id on fork PRs.
    let is_fork = pr["head"]["repo"]["id"].as_i64().unwrap_or(0) != repo_id;
    let labels: Vec<&str> = pr["labels"]
        .as_array()
        .map(|ls| ls.iter().filter_map(|l| l["name"].as_str()).collect())
        .unwrap_or_default();

    let projects: Vec<Project> = match sqlx::query_as(
        "SELECT * FROM project
         WHERE repo_id = $1 AND repo_provider = 'github' AND status = 'active'",
    )
    .bind(repo_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "pull_request webhook: project lookup failed");
            return;
        }
    };

    // `unlabeled` and `converted_to_draft` can move a PR outside a
    // project's preview policy — tear down rather than leave stale
    // previews serving.
    if action == "closed" {
        for project in &projects {
            teardown_preview(state, project, &branch).await;
        }
        return;
    }

    let commit = CommitInfo {
        sha,
        author: pr["user"]["login"].as_str().unwrap_or("").to_string(),
        message: format!("{} (PR #{number})", pr["title"].as_str().unwrap_or("")),
        timestamp: pr["updated_at"].as_str().map(String::from),
    };

    for project in projects {
        let rules = project
            .config
            .get("deployment_rules")
            .cloned()
            .unwrap_or(json!({}));
        if !preview_allowed(&rules, is_draft, is_fork, &labels) {
            if matches!(action, "unlabeled" | "converted_to_draft") {
                teardown_preview(state, &project, &branch).await;
            }
            continue;
        }
        if matches!(action, "unlabeled" | "converted_to_draft") {
            continue;
        }

        // Cancel superseded preview builds on the same ref — rapid pushes
        // should not queue every SHA. `cancel_superseded=false` opts out.
        if rules
            .get("cancel_superseded")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
        {
            cancel_pending(state, &project, &branch).await;
        }

        match deploy::create(
            &state.db,
            &state.bus,
            &state.crypto,
            &project,
            &branch,
            &commit,
            "webhook",
            None,
            Some(json!({ "pr_number": number, "pr_fork": is_fork })),
        )
        .await
        {
            Ok(dep) => {
                tracing::info!(
                    deployment_id = dep.id,
                    project_id = project.id,
                    "preview deployment created from pull_request"
                );
                if let Some(level) = comment_verbosity(&rules) {
                    post_preview_comment(state, &project, &branch, number, level, &dep).await;
                }
            }
            Err(e) => {
                tracing::warn!(project_id = project.id, error = %e, "pull_request: deployment create failed")
            }
        }
    }
}

/// `@runway deploy` / `/runway deploy` PR comments — first word only,
/// so prose mentioning the command doesn't redeploy.
fn is_deploy_command(body: &str) -> bool {
    let cmd = body.trim().to_lowercase();
    cmd.starts_with("@runway deploy") || cmd.starts_with("/runway deploy")
}

/// Comment-command redeploy: find the latest deployment tagged with
/// this PR number on any project watching the repo, redeploy it.
async fn rerun_pr_preview(state: &AppState, data: &Value, pr_number: i64) {
    let repo_id = data["repository"]["id"].as_i64().unwrap_or(0);
    let projects: Vec<Project> = sqlx::query_as(
        "SELECT * FROM project WHERE repo_id = $1 AND repo_provider = 'github' AND status = 'active'",
    )
    .bind(repo_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for project in projects {
        let latest: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM deployment
             WHERE project_id = $1 AND config->>'pr_number' = $2
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&project.id)
        .bind(pr_number.to_string())
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
        if let Some((dep_id,)) = latest {
            rerun_deployment(state, &dep_id).await;
        }
    }
}

/// check_run.rerequested — redeploy the deployment named by
/// `external_id` from its recorded source. No project-config policy is
/// applied: a human clicked Re-run.
async fn rerun_deployment(state: &AppState, deployment_id: &str) {
    let dep = match deploy::get(&state.db, deployment_id).await {
        Ok(Some(d)) => d,
        _ => return,
    };
    let project: Option<Project> = sqlx::query_as("SELECT * FROM project WHERE id = $1")
        .bind(&dep.project_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    let Some(project) = project else { return };
    let commit = CommitInfo {
        sha: dep.commit_sha.clone(),
        message: dep
            .commit_meta
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        author: dep
            .commit_meta
            .get("author")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        timestamp: None,
    };
    let mut extra = json!({});
    if let Some(a) = dep.config.get("source_archive") {
        extra["source_archive"] = a.clone();
    }
    if let Some(n) = dep.config.get("pr_number") {
        extra["pr_number"] = n.clone();
    }
    match deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        &project,
        &dep.branch,
        &commit,
        "webhook",
        None,
        Some(extra),
    )
    .await
    {
        Ok(new_dep) => tracing::info!(
            deployment_id = new_dep.id,
            "check_run rerequested → redeploy"
        ),
        Err(e) => tracing::warn!(error = %e, "check_run rerequested: redeploy failed"),
    }
}

/// Whether a PR event may produce a preview under the project's rules.
/// Defaults match prior behaviour (drafts deploy, previews on) except
/// forks, which now require explicit opt-in — their code is untrusted
/// and previews would run it with project env vars.
fn preview_allowed(
    rules: &serde_json::Value,
    is_draft: bool,
    is_fork: bool,
    labels: &[&str],
) -> bool {
    if rules.get("auto_deploy").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    if rules.get("preview_prs").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    if is_draft && rules.get("preview_drafts").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    if is_fork && rules.get("preview_forks").and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    // Label gate: when set, the PR must carry the label.
    if let Some(gate) = rules.get("preview_label").and_then(|v| v.as_str()) {
        if !gate.is_empty() && !labels.contains(&gate) {
            return false;
        }
    }
    true
}

/// `preview_comment`: bool or "off"|"minimal"|"full". None = no comment.
fn comment_verbosity(rules: &serde_json::Value) -> Option<&'static str> {
    match rules.get("preview_comment") {
        Some(serde_json::Value::Bool(b)) => b.then_some("full"),
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "off" => None,
            "minimal" => Some("minimal"),
            _ => Some("full"),
        },
        Some(_) => None,
        None => Some("full"), // default: comment, full verbosity
    }
}

/// Cancel unconcluded deployments on this project+branch that have not
/// reached the finalize stage (superseded by the incoming commit).
async fn cancel_pending(state: &AppState, project: &Project, branch: &str) {
    let pending: Vec<runway_core::models::Deployment> = sqlx::query_as(
        "SELECT * FROM deployment WHERE project_id = $1 AND branch = $2
         AND conclusion IS NULL AND status IN ('prepare','deploy')",
    )
    .bind(&project.id)
    .bind(branch)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let docker = match runway_core::docker::connect(&state.settings) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "superseded cancel: docker connect failed");
            return;
        }
    };
    for dep in pending {
        if let Err(e) = deploy::cancel(&state.db, &state.bus, &state.settings, &docker, &dep).await
        {
            tracing::warn!(deployment_id = dep.id, error = %e, "superseded cancel failed");
        }
    }
}

/// Upserted PR comment with the deterministic branch preview URL.
/// `level` is "minimal" (URL only) or "full" (URL + state + commit).
async fn post_preview_comment(
    state: &AppState,
    project: &Project,
    branch: &str,
    number: i64,
    level: &str,
    dep: &runway_core::models::Deployment,
) {
    let (Some(gh), Some(installation_id)) =
        (state.github.if_configured(), project.github_installation_id)
    else {
        return;
    };
    let Ok(token) = gh
        .installation_token(&state.db, &state.crypto, installation_id)
        .await
    else {
        return;
    };
    let slug = project.slug.clone().unwrap_or_else(|| project.id.clone());
    let host = format!(
        "{slug}-branch-{}.{}",
        runway_core::slugify::branch_slug(branch),
        state.settings.deploy_domain
    );
    let url = format!("{}://{}", state.settings.url_scheme, host);
    let body = if level == "minimal" {
        format!("**Runway preview** — {url}")
    } else {
        format!(
            "**Runway preview** — [{url}]({url})\n\n\
             `{branch}` @ `{}` — building; the URL goes live when it finishes.",
            &dep.commit_sha[..dep.commit_sha.len().min(7)],
        )
    };
    let marker = format!("<!-- runway-preview:{} -->", project.id);
    if let Err(e) = gh
        .upsert_issue_comment(&token, &project.repo_full_name, number, &marker, &body)
        .await
    {
        tracing::warn!(project_id = project.id, error = %e, "preview comment post failed");
    }
}

/// PR closed/merged: drop the branch + preview-env aliases (removes
/// routing) and enqueue container teardown for the branch's deployments.
async fn teardown_preview(state: &AppState, project: &Project, branch: &str) {
    if let Err(e) = sqlx::query(
        "DELETE FROM alias a USING deployment d
         WHERE a.deployment_id = d.id AND d.project_id = $1
           AND ((a.type = 'branch' AND a.value = $2)
             OR (a.type = 'environment_id' AND a.value IN
                 (SELECT DISTINCT environment_id FROM deployment
                  WHERE project_id = $1 AND branch = $2)))",
    )
    .bind(&project.id)
    .bind(branch)
    .execute(&state.db)
    .await
    {
        tracing::warn!(project_id = project.id, error = %e, "preview alias teardown failed");
        return;
    }
    if let Err(e) =
        runway_core::traefik::update_project_config(&state.db, project, &state.settings, &[]).await
    {
        tracing::warn!(project_id = project.id, error = %e, "preview traefik rewrite failed");
    }
    let dep_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM deployment WHERE project_id = $1 AND branch = $2
         AND container_id IS NOT NULL AND container_status != 'removed'",
    )
    .bind(&project.id)
    .bind(branch)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for dep_id in dep_ids {
        let _ = deploy::enqueue(
            &state.db,
            "delete_container",
            json!({ "deployment_id": dep_id }),
            0,
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Repo browsing — app-level. The instance lists its own installations
// (app JWT) and their repos (installation token); no user OAuth needed.
// ---------------------------------------------------------------------------

/// Installations of the registered app, plus the install URL.
pub async fn installations(_user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    if !state.github.configured() {
        return Ok(Json(json!({
            "installations": [],
            "install_url": null,
            "configured": false,
        }))
        .into_response());
    }
    let installs = state
        .github
        .app_installations()
        .await
        .map_err(ApiError::internal)?;
    let mapped: Vec<Value> = installs
        .iter()
        .map(|i| {
            json!({
                "id": i["id"],
                "account": i["account"]["login"].as_str().unwrap_or(""),
            })
        })
        .collect();
    Ok(Json(json!({
        "installations": mapped,
        "install_url": state.github.install_url(),
        "configured": true,
    }))
    .into_response())
}

pub async fn installation_repos(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(installation_id): Path<i64>,
) -> ApiResult<Response> {
    if !state.github.configured() {
        return Err(ApiError::bad_request("GitHub App is not configured"));
    }
    let token = state
        .github
        .installation_token(&state.db, &state.crypto, installation_id)
        .await
        .map_err(ApiError::internal)?;
    let repos = state
        .github
        .installation_repositories(&token)
        .await
        .map_err(ApiError::internal)?;
    let mapped: Vec<Value> = repos
        .into_iter()
        .map(|r| {
            json!({
                "id": r["id"],
                "full_name": r["full_name"],
                "default_branch": r["default_branch"],
                "private": r["private"],
            })
        })
        .collect();
    Ok(Json(json!({ "repositories": mapped })).into_response())
}

// ---------------------------------------------------------------------------
// App registration — GitHub's manifest flow. One click in Settings →
// GitHub creates the app with our permissions/webhook → callback
// exchanges the code → credentials stored encrypted in `github_app`,
// hot-patched into the shared service (no restart).
// ---------------------------------------------------------------------------

/// `GET /api/v1/github/app/status` — configured?, slug, install URL.
pub async fn app_status(_user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    Ok(Json(json!({
        "configured": state.github.configured(),
        "source": if state.settings.github_app_configured() { "env" } else { "db" },
        "slug": state.github.slug(),
        "install_url": state.github.install_url(),
        "web_base": state.github.web_base,
    }))
    .into_response())
}

/// True when a hostname is reachable from the public Internet —
/// i.e. GitHub could deliver webhooks to it.
fn is_public_hostname(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let local = [
        "localhost",
        "local",
        "test",
        "internal",
        "invalid",
        "lan",
        "home",
    ];
    !local
        .iter()
        .any(|t| host == *t || host.ends_with(&format!(".{t}")))
        && host.contains('.')
}

/// `GET /api/v1/github/app/register` — admin only. Returns an
/// auto-submitting HTML form that POSTs the manifest to GitHub; GitHub
/// then redirects to `app_callback` with `?code=`.
pub async fn app_register(
    user: AuthUser,
    State(state): State<AppState>,
    jar: CookieJar,
) -> ApiResult<Response> {
    crate::routes::admin::require_superadmin(&user)?;
    if state.settings.github_app_configured() {
        return Err(ApiError::bad_request(
            "GitHub App already configured via environment",
        ));
    }
    let scheme = &state.settings.url_scheme;
    let host = &state.settings.app_hostname;
    let base = format!("{scheme}://{host}");
    // GitHub validates the webhook URL is publicly reachable at registration —
    // omit it entirely on localhost/intranet installs (push events can't reach
    // them anyway) rather than failing manifest validation.
    let mut manifest = json!({
        "name": format!("runway-{}", host.replace('.', "-")),
        "url": base,
        "redirect_url": format!("{base}/api/v1/github/app/callback"),
        "callback_urls": [base],
        "setup_url": format!("{base}/settings"),
        "public": false,
        "default_permissions": {
            "contents": "read",
            "metadata": "read",
            "pull_requests": "read",
            "statuses": "write",
            "checks": "write",
            "issues": "write",
        },
    });
    if is_public_hostname(host) {
        manifest["hook_attributes"] = json!({
            "url": format!("{base}/api/github/webhook"),
            "active": true,
        });
        manifest["default_events"] = json!([
            "push",
            "pull_request",
            "repository",
            "check_run",
            "issue_comment"
        ]);
    }
    let state_token = runway_core::slugify::token_hex(16);
    let cookie = Cookie::build(("gh_app_state", state_token.clone()))
        .path("/")
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();
    let manifest_json = serde_json::to_string(&manifest)
        .map_err(ApiError::internal)?
        .replace('\'', "&#39;");
    let html = format!(
        r#"<!doctype html><html><body>
<p>Redirecting to GitHub…</p>
<form id="f" method="post" action="{}/settings/apps/new">
  <input type="hidden" name="manifest" value='{}'>
  <input type="hidden" name="state" value="{}">
</form>
<script>document.getElementById("f").submit()</script>
</body></html>"#,
        state.github.web_base, manifest_json, state_token
    );
    Ok((jar.add(cookie), axum::response::Html(html)).into_response())
}

#[derive(Deserialize)]
pub struct AppCallbackParams {
    code: String,
    state: String,
}

/// `GET /api/v1/github/app/callback` — exchange the manifest code,
/// encrypt + store credentials, configure the shared service.
pub async fn app_callback(
    user: AuthUser,
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<AppCallbackParams>,
) -> ApiResult<Response> {
    crate::routes::admin::require_superadmin(&user)?;
    let expected = jar.get("gh_app_state").map(|c| c.value().to_string());
    if expected.as_deref() != Some(params.state.as_str()) {
        return Err(ApiError::unauthorized("invalid app registration state"));
    }
    let creds = state
        .github
        .exchange_manifest_code(&params.code)
        .await
        .map_err(|e| ApiError::bad_request(format!("manifest exchange failed: {e}")))?;
    let app_id = match &creds.id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err(ApiError::bad_request("manifest returned no app id")),
    };
    let webhook_secret = creds
        .webhook_secret
        .clone()
        .ok_or_else(|| ApiError::bad_request("manifest returned no webhook secret"))?;
    let client_secret_enc = match &creds.client_secret {
        Some(s) => Some(state.crypto.encrypt(s).map_err(ApiError::internal)?),
        None => None,
    };
    sqlx::query(
        "INSERT INTO github_app
           (id, app_id, slug, name, client_id, client_secret_enc, pem_enc,
            webhook_secret_enc, html_url, created_by_user_id)
         VALUES (1, $1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (id) DO UPDATE SET
           app_id = $1, slug = $2, name = $3, client_id = $4,
           client_secret_enc = $5, pem_enc = $6, webhook_secret_enc = $7,
           html_url = $8, created_by_user_id = $9, updated_at = now()",
    )
    .bind(&app_id)
    .bind(&creds.slug)
    .bind(&creds.name)
    .bind(&creds.client_id)
    .bind(&client_secret_enc)
    .bind(
        state
            .crypto
            .encrypt(&creds.pem)
            .map_err(ApiError::internal)?,
    )
    .bind(
        state
            .crypto
            .encrypt(&webhook_secret)
            .map_err(ApiError::internal)?,
    )
    .bind(&creds.html_url)
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    // Env config wins when present — only hot-patch when it isn't.
    if !state.settings.github_app_configured() {
        state.github.configure(runway_core::github::RegisteredApp {
            app_id,
            slug: creds.slug,
            private_key: creds.pem,
            webhook_secret,
        });
    }
    let jar = jar.remove(
        Cookie::build(("gh_app_state", String::new()))
            .path("/")
            .build(),
    );
    Ok((jar, Redirect::to("/settings")).into_response())
}

#[cfg(test)]
mod tests {
    use super::{comment_verbosity, is_public_hostname, preview_allowed};

    #[test]
    fn public_hostname() {
        assert!(is_public_hostname("runway.example.com"));
        assert!(is_public_hostname("deploys.example.co.uk:443"));
        assert!(!is_public_hostname("runway.localhost"));
        assert!(!is_public_hostname("localhost"));
        assert!(!is_public_hostname("runway.lan"));
        assert!(!is_public_hostname("192.168.1.10"));
        assert!(!is_public_hostname("runway")); // single-label
        assert!(!is_public_hostname(""));
    }
}

#[test]
fn previews_on_by_default() {
    assert!(preview_allowed(&json!({}), false, false, &[]));
}

#[test]
fn master_switch_off() {
    assert!(!preview_allowed(
        &json!({ "auto_deploy": false }),
        false,
        false,
        &[]
    ));
    assert!(!preview_allowed(
        &json!({ "preview_prs": false }),
        false,
        false,
        &[]
    ));
}

#[test]
fn drafts_deploy_unless_opted_out() {
    assert!(preview_allowed(&json!({}), true, false, &[]));
    assert!(!preview_allowed(
        &json!({ "preview_drafts": false }),
        true,
        false,
        &[]
    ));
}

#[test]
fn forks_require_opt_in() {
    assert!(!preview_allowed(&json!({}), false, true, &[]));
    assert!(preview_allowed(
        &json!({ "preview_forks": true }),
        false,
        true,
        &[]
    ));
}

#[test]
fn label_gate() {
    let r = json!({ "preview_label": "deploy-preview" });
    assert!(!preview_allowed(&r, false, false, &[]));
    assert!(preview_allowed(&r, false, false, &["deploy-preview"]));
    assert!(!preview_allowed(&r, false, false, &["other"]));
}

#[test]
fn comment_verbosity_levels() {
    assert_eq!(comment_verbosity(&json!({})), Some("full"));
    assert_eq!(
        comment_verbosity(&json!({ "preview_comment": false })),
        None
    );
    assert_eq!(
        comment_verbosity(&json!({ "preview_comment": "off" })),
        None
    );
    assert_eq!(
        comment_verbosity(&json!({ "preview_comment": "minimal" })),
        Some("minimal")
    );
}

#[test]
fn deploy_command_parsing() {
    assert!(is_deploy_command("@runway deploy"));
    assert!(is_deploy_command("  /runway deploy now"));
    assert!(!is_deploy_command("try @runway deploy"));
    assert!(!is_deploy_command("@runway redeploy"));
    assert!(!is_deploy_command(""));
}
