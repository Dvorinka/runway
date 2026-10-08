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
                "created" => {
                    if state.github.configured() {
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

    for project in projects {
        // Deployment rules — port of the webhook filters.
        let rules = project
            .config
            .get("deployment_rules")
            .cloned()
            .unwrap_or(json!({}));
        if rules.get("auto_deploy").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let deploy_branches = rules
            .get("deploy_branches")
            .and_then(|v| v.as_str())
            .unwrap_or("main,master");
        if deploy_branches.trim() != "*" {
            let allowed: Vec<&str> = deploy_branches
                .split(',')
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .collect();
            if !allowed.contains(&branch.as_str()) {
                continue;
            }
        }
        let ignored: Vec<&str> = rules
            .get("ignored_authors")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .collect();
        if ignored.contains(&commit.author.as_str()) {
            continue;
        }
        if rules.get("skip_merge_commits").and_then(|v| v.as_bool()) == Some(true)
            && commit.message.starts_with("Merge")
        {
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
/// PR head branch. Branch allowlists don't apply — previews are the
/// point. `auto_deploy=false` still opts the project out.
async fn handle_pull_request(state: &AppState, data: &Value) {
    let action = data["action"].as_str().unwrap_or("");
    if !matches!(action, "opened" | "synchronize" | "reopened") {
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
    let commit = CommitInfo {
        sha,
        author: pr["user"]["login"].as_str().unwrap_or("").to_string(),
        message: format!("{} (PR #{number})", pr["title"].as_str().unwrap_or("")),
        timestamp: pr["updated_at"].as_str().map(String::from),
    };

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

    for project in projects {
        let rules = project
            .config
            .get("deployment_rules")
            .cloned()
            .unwrap_or(json!({}));
        if rules.get("auto_deploy").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if rules.get("preview_prs").and_then(|v| v.as_bool()) == Some(false) {
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
                "preview deployment created from pull_request"
            ),
            Err(e) => {
                tracing::warn!(project_id = project.id, error = %e, "pull_request: deployment create failed")
            }
        }
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
    let manifest = json!({
        "name": format!("runway-{}", host.replace('.', "-")),
        "url": base,
        "hook_attributes": {
            "url": format!("{base}/api/github/webhook"),
            "active": true,
        },
        "redirect_url": format!("{base}/api/v1/github/app/callback"),
        "callback_urls": [base],
        "setup_url": format!("{base}/settings"),
        "public": false,
        "default_permissions": {
            "contents": "read",
            "metadata": "read",
            "pull_requests": "read",
            "statuses": "write",
        },
        "default_events": ["push", "pull_request", "repository"],
    });
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
