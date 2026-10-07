//! GitHub integration routes: webhook receiver + repo browsing for
//! project creation (uses the user's OAuth token).

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use runway_core::deploy::{self, CommitInfo};
use runway_core::github::GithubService;
use runway_core::models::{Project, UserIdentity};

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
    let Some(secret) = &state.settings.github_app_webhook_secret else {
        return Err(ApiError::bad_request("webhook secret not configured"));
    };
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !GithubService::verify_webhook(secret, &body, signature) {
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
                    if let Some(gh) = &state.github {
                        if let Ok((token, expires)) =
                            gh.installation_access_token(installation_id).await
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
            "pull_request",
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
// Repo browsing (OAuth token → installations → repos)
// ---------------------------------------------------------------------------

/// Installations the logged-in user can see, plus the app install URL.
pub async fn installations(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let token = user_oauth_token(&state, user.user.id).await?;
    let gh = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth not configured"))?;
    let installs = gh
        .user_installations(&token)
        .await
        .map_err(ApiError::internal)?;
    let install_url = state
        .settings
        .github_app_name
        .as_ref()
        .map(|n| format!("https://github.com/apps/{n}/installations/new"));
    Ok(Json(json!({
        "installations": installs,
        "install_url": install_url,
    }))
    .into_response())
}

pub async fn installation_repos(
    user: AuthUser,
    State(state): State<AppState>,
    Path(installation_id): Path<i64>,
) -> ApiResult<Response> {
    let token = user_oauth_token(&state, user.user.id).await?;
    let gh = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth not configured"))?;
    let repos = gh
        .installation_repositories_for_user(&token, installation_id)
        .await
        .map_err(ApiError::internal)?;
    // Only repos the user can push to are deployable.
    let writable: Vec<Value> = repos
        .into_iter()
        .filter(|r| r["permissions"]["push"].as_bool() == Some(true))
        .map(|r| {
            json!({
                "id": r["id"],
                "full_name": r["full_name"],
                "default_branch": r["default_branch"],
                "private": r["private"],
            })
        })
        .collect();
    Ok(Json(json!({ "repositories": writable })).into_response())
}

/// The user's decrypted GitHub OAuth token.
async fn user_oauth_token(state: &AppState, user_id: i64) -> ApiResult<String> {
    let identity: Option<UserIdentity> =
        sqlx::query_as("SELECT * FROM user_identity WHERE user_id = $1 AND provider = 'github'")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(identity) = identity else {
        return Err(ApiError::bad_request("no GitHub identity linked"));
    };
    let Some(enc) = identity.access_token else {
        return Err(ApiError::bad_request("no GitHub access token stored"));
    };
    state.crypto.decrypt(&enc).map_err(ApiError::from)
}
