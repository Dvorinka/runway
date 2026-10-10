//! Gitea/GitLab/Bitbucket provider connections + push webhooks — port of
//! devpush `routers/gitea.py` (gitlab/bitbucket equivalents).
//!
//! - `POST   /api/v1/git/{provider}/connect`               — verify + store token
//! - `GET    /api/v1/git/{provider}/connections`           — list (tokens masked)
//! - `DELETE /api/v1/git/{provider}/connections/{id}`      — remove
//! - `GET    /api/v1/git/{provider}/connections/{id}/repos`
//! - `GET    /api/v1/git/{provider}/connections/{id}/branches/{*full}`
//! - `POST   /api/gitea/webhook`   — X-Gitea-Signature (hex HMAC-SHA256)
//! - `POST   /api/gitlab/webhook`  — X-Gitlab-Token + Push Hook event
//! - `POST   /api/bitbucket/webhook` — X-Event-Key: repo:push; the
//!   payload's sha is never trusted — the branch head is resolved via
//!   the connection's API credentials before deploying.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::deploy::{self, CommitInfo};
use runway_core::git_providers::{self, Client};
use runway_core::models::Project;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

fn provider_kind(provider: &str) -> ApiResult<&'static str> {
    match provider {
        "gitea" => Ok("gitea"),
        "forgejo" => Ok("forgejo"),
        "gitlab" => Ok("gitlab"),
        "bitbucket" => Ok("bitbucket"),
        _ => Err(ApiError::not_found("provider")),
    }
}

fn provider_table(provider: &str) -> ApiResult<&'static str> {
    Ok(match provider_kind(provider)? {
        "gitea" | "forgejo" => "gitea_connection",
        "gitlab" => "gitlab_connection",
        _ => "bitbucket_connection",
    })
}

/// Load + decrypt a connection owned by the user.
async fn user_connection(
    state: &AppState,
    user_id: i64,
    provider: &str,
    conn_id: i64,
) -> ApiResult<git_providers::Connection> {
    let conn = git_providers::connection(&state.db, &state.crypto, provider, conn_id)
        .await?
        .ok_or_else(|| ApiError::not_found("connection"))?;
    let owner: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT user_id FROM {} WHERE id = $1",
        provider_table(provider)?
    ))
    .bind(conn_id)
    .fetch_optional(&state.db)
    .await?;
    if owner != Some(user_id) {
        return Err(ApiError::not_found("connection"));
    }
    Ok(conn)
}

fn client_for(provider: &str, conn: git_providers::Connection) -> Client {
    Client::new(provider, conn)
}

#[derive(Deserialize)]
pub struct ConnectBody {
    /// gitea/gitlab: instance URL (e.g. `https://git.example.com`).
    /// gitlab defaults to https://gitlab.com.
    pub base_url: Option<String>,
    /// bitbucket: workspace slug.
    pub workspace: Option<String>,
    pub token: String,
}

/// Verify the token against the provider's `/user` endpoint and persist
/// it encrypted. Username is read back from the provider (not trusted
/// from the client).
pub async fn connect(
    user: AuthUser,
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(body): Json<ConnectBody>,
) -> ApiResult<Response> {
    let provider = provider_kind(&provider)?;
    if body.token.trim().is_empty() {
        return Err(ApiError::bad_request("token required"));
    }

    // Probe the token before storing — a bad token fails fast here.
    let base_url = match provider {
        "gitea" | "forgejo" => body
            .base_url
            .clone()
            .filter(|b| !b.trim().is_empty())
            .ok_or_else(|| ApiError::bad_request("base_url required"))?
            .trim_end_matches('/')
            .to_string(),
        "gitlab" => body
            .base_url
            .clone()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| "https://gitlab.com".into())
            .trim_end_matches('/')
            .to_string(),
        "bitbucket" => "https://api.bitbucket.org".into(),
        _ => unreachable!(),
    };
    let probe = probe_client(provider, &base_url, &body);
    let account = probe
        .verify_user()
        .await
        .map_err(|_| ApiError::bad_request("token rejected by provider"))?;
    let username = match provider {
        "gitea" | "forgejo" => account["login"].as_str(),
        "gitlab" => account["username"].as_str(),
        _ => account["nickname"]
            .as_str()
            .or_else(|| account["username"].as_str()),
    }
    .unwrap_or("")
    .to_string();

    let ciphertext = state.crypto.encrypt(&body.token)?;
    let table = provider_table(provider)?;
    let row: (i64,) = if provider == "bitbucket" {
        let workspace = body
            .workspace
            .clone()
            .filter(|w| !w.trim().is_empty())
            .ok_or_else(|| ApiError::bad_request("workspace required"))?;
        sqlx::query_as(&format!(
            "INSERT INTO {table} (user_id, workspace, token)
             VALUES ($1,$2,$3)
             ON CONFLICT (user_id, workspace) DO UPDATE SET
                token = EXCLUDED.token, updated_at = now()
             RETURNING id"
        ))
        .bind(user.user.id)
        .bind(&workspace)
        .bind(&ciphertext)
        .fetch_one(&state.db)
        .await?
    } else {
        sqlx::query_as(&format!(
            "INSERT INTO {table} (user_id, base_url, username, token)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (user_id, base_url) DO UPDATE SET
                username = EXCLUDED.username, token = EXCLUDED.token,
                updated_at = now()
             RETURNING id"
        ))
        .bind(user.user.id)
        .bind(&base_url)
        .bind(&username)
        .bind(&ciphertext)
        .fetch_one(&state.db)
        .await?
    };

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": row.0,
            "provider": provider,
            "base_url": base_url,
            "username": username,
        })),
    )
        .into_response())
}

/// Build a probe `Client` from a not-yet-persisted token.
fn probe_client(provider: &str, base_url: &str, body: &ConnectBody) -> Client {
    let username = if provider == "bitbucket" {
        body.workspace.clone().unwrap_or_default()
    } else {
        String::new()
    };
    Client::new(
        provider,
        git_providers::Connection {
            id: 0,
            base_url: base_url.to_string(),
            username,
            token: body.token.clone(),
        },
    )
}

/// List the user's connections for a provider — tokens never leave core.
pub async fn list_connections(
    user: AuthUser,
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<Response> {
    let table = provider_table(&provider)?;
    let rows: Vec<(i64, Option<String>, String, String)> = sqlx::query_as(&format!(
        "SELECT id, {base} AS base_url, {name} AS username, created_at::text
         FROM {table} WHERE user_id = $1 ORDER BY id",
        base = if provider == "bitbucket" {
            "'https://api.bitbucket.org'"
        } else {
            "base_url"
        },
        name = if provider == "bitbucket" {
            "workspace"
        } else {
            "username"
        },
    ))
    .bind(user.user.id)
    .fetch_all(&state.db)
    .await?;
    let conns: Vec<Value> = rows
        .into_iter()
        .map(|(id, base_url, username, created_at)| {
            json!({
                "id": id,
                "base_url": base_url,
                "username": username,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "connections": conns })).into_response())
}

/// Delete a connection — fails 409 while projects still reference it.
pub async fn delete_connection(
    user: AuthUser,
    State(state): State<AppState>,
    Path((provider, conn_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    user_connection(&state, user.user.id, &provider, conn_id).await?;
    let table = provider_table(&provider)?;
    sqlx::query(&format!("DELETE FROM {table} WHERE id = $1"))
        .bind(conn_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            if e.to_string().contains("foreign key") {
                ApiError::conflict("connection is in use by a project")
            } else {
                ApiError::from(e)
            }
        })?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Repos visible to the connection token.
pub async fn list_repos(
    user: AuthUser,
    State(state): State<AppState>,
    Path((provider, conn_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let provider: &'static str = provider_kind(&provider)?;
    let conn = user_connection(&state, user.user.id, provider, conn_id).await?;
    let repos = client_for(provider, conn)
        .list_repos()
        .await
        .map_err(|e| ApiError::bad_request(format!("provider error: {e}")))?;
    // Normalize to {id, full_name, name, private, default_branch}.
    let out: Vec<Value> = repos
        .iter()
        .map(|r| match provider {
            "gitlab" => json!({
                "id": r["id"].as_i64().unwrap_or(0),
                "full_name": r["path_with_namespace"].as_str().unwrap_or(""),
                "name": r["name"].as_str().unwrap_or(""),
                "private": r["visibility"].as_str() == Some("private"),
                "default_branch": r["default_branch"].as_str().unwrap_or("main"),
            }),
            "bitbucket" => json!({
                "id": r["uuid"].as_str().unwrap_or(""),
                "full_name": r["full_name"].as_str().unwrap_or(""),
                "name": r["name"].as_str().unwrap_or(""),
                "private": r["is_private"].as_bool().unwrap_or(false),
                "default_branch": r["mainbranch"]["name"].as_str().unwrap_or("main"),
            }),
            _ => json!({
                "id": r["id"].as_i64().unwrap_or(0),
                "full_name": r["full_name"].as_str().unwrap_or(""),
                "name": r["name"].as_str().unwrap_or(""),
                "private": r["private"].as_bool().unwrap_or(false),
                "default_branch": r["default_branch"].as_str().unwrap_or("main"),
            }),
        })
        .collect();
    Ok(Json(json!({ "repositories": out })).into_response())
}

/// Branches for `owner/repo` (`{*full}` captures nested gitlab groups).
pub async fn list_branches(
    user: AuthUser,
    State(state): State<AppState>,
    Path((provider, conn_id, full)): Path<(String, i64, String)>,
) -> ApiResult<Response> {
    let provider = provider_kind(&provider)?;
    let conn = user_connection(&state, user.user.id, provider, conn_id).await?;
    let (owner, repo) = full
        .rsplit_once('/')
        .ok_or_else(|| ApiError::bad_request("expected owner/repo"))?;
    let branches = client_for(provider, conn)
        .list_branches(owner, repo)
        .await
        .map_err(|e| ApiError::bad_request(format!("provider error: {e}")))?;
    let names: Vec<&str> = branches.iter().filter_map(|b| b["name"].as_str()).collect();
    Ok(Json(json!({ "branches": names })).into_response())
}

// ---------- inbound webhooks ----------

/// `POST /api/gitea/webhook` — hex-HMAC signed, `X-Gitea-Event: push`.
/// `POST /api/forgejo/webhook` — same payload shape, `X-Forgejo-*` headers.
pub async fn gitea_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let Some(secret) = &state.settings.gitea_webhook_secret else {
        return Err(ApiError::bad_request("gitea webhook not configured"));
    };
    let signature = headers
        .get("x-gitea-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !git_providers::verify_gitea_signature(secret, &body, signature) {
        return Err(ApiError::unauthorized("invalid signature"));
    }
    let event = headers
        .get("x-gitea-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid payload"))?;
    if event == "push" {
        let repo = &data["repository"];
        let repo_id = repo["id"].as_i64().unwrap_or(0);
        // Base URL = html_url minus `/owner/repo` (devpush derivation).
        let base_url = repo["html_url"]
            .as_str()
            .unwrap_or("")
            .rsplitn(3, '/')
            .last()
            .unwrap_or("")
            .to_string();
        let last = data["commits"]
            .as_array()
            .and_then(|c| c.last())
            .cloned()
            .unwrap_or(json!({}));
        let commit = CommitInfo {
            sha: data["after"].as_str().unwrap_or_default().into(),
            author: data["pusher"]["login"]
                .as_str()
                .or_else(|| data["pusher"]["username"].as_str())
                .unwrap_or_default()
                .into(),
            message: last["message"].as_str().unwrap_or_default().into(),
            timestamp: last["timestamp"].as_str().map(String::from),
        };
        provider_push(&state, "gitea", repo_id, &base_url, &data, commit).await;
    }
    Ok(StatusCode::OK.into_response())
}

/// `POST /api/forgejo/webhook` — Forgejo speaks the Gitea webhook
/// dialect with `X-Forgejo-Signature` / `X-Forgejo-Event` headers.
/// Shares the gitea webhook secret setting.
pub async fn forgejo_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let Some(secret) = &state.settings.gitea_webhook_secret else {
        return Err(ApiError::bad_request("forgejo webhook not configured"));
    };
    let signature = headers
        .get("x-forgejo-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !git_providers::verify_gitea_signature(secret, &body, signature) {
        return Err(ApiError::unauthorized("invalid signature"));
    }
    let event = headers
        .get("x-forgejo-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid payload"))?;
    if event == "push" {
        let repo = &data["repository"];
        let repo_id = repo["id"].as_i64().unwrap_or(0);
        let base_url = repo["html_url"]
            .as_str()
            .unwrap_or("")
            .rsplitn(3, '/')
            .last()
            .unwrap_or("")
            .to_string();
        let last = data["commits"]
            .as_array()
            .and_then(|c| c.last())
            .cloned()
            .unwrap_or(json!({}));
        let commit = CommitInfo {
            sha: data["after"].as_str().unwrap_or_default().into(),
            author: data["pusher"]["login"]
                .as_str()
                .or_else(|| data["pusher"]["username"].as_str())
                .unwrap_or_default()
                .into(),
            message: last["message"].as_str().unwrap_or_default().into(),
            timestamp: last["timestamp"].as_str().map(String::from),
        };
        provider_push(&state, "forgejo", repo_id, &base_url, &data, commit).await;
    }
    Ok(StatusCode::OK.into_response())
}

/// `POST /api/gitlab/webhook` — shared-token auth, `Push Hook` events.
pub async fn gitlab_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let Some(secret) = &state.settings.gitlab_webhook_secret else {
        return Err(ApiError::bad_request("gitlab webhook not configured"));
    };
    let token = headers
        .get("x-gitlab-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if token.is_empty() || !git_providers::verify_gitlab_token(secret, token) {
        return Err(ApiError::unauthorized("invalid token"));
    }
    let event = headers
        .get("x-gitlab-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid payload"))?;
    if event == "Push Hook" {
        let proj = &data["project"];
        let repo_id = proj["id"].as_i64().unwrap_or(0);
        // `web_url` = base_url + "/" + path_with_namespace.
        let web_url = proj["web_url"].as_str().unwrap_or("");
        let path = proj["path_with_namespace"].as_str().unwrap_or("");
        let base_url = web_url
            .strip_suffix(path)
            .map(|b| b.trim_end_matches('/').to_string())
            .unwrap_or_default();
        let last = data["commits"]
            .as_array()
            .and_then(|c| c.last())
            .cloned()
            .unwrap_or(json!({}));
        let commit = CommitInfo {
            sha: data["checkout_sha"].as_str().unwrap_or_default().into(),
            author: data["user_username"]
                .as_str()
                .or_else(|| data["user_name"].as_str())
                .unwrap_or_default()
                .into(),
            message: last["message"].as_str().unwrap_or_default().into(),
            timestamp: last["timestamp"].as_str().map(String::from),
        };
        provider_push(&state, "gitlab", repo_id, &base_url, &data, commit).await;
    }
    Ok(StatusCode::OK.into_response())
}

/// Shared push handling — match active projects on
/// (provider, repo_id, base_url), apply deployment rules, create +
/// enqueue deployments. Mirrors `handle_push` in github.rs.
async fn provider_push(
    state: &AppState,
    provider: &str,
    repo_id: i64,
    base_url: &str,
    data: &Value,
    commit: CommitInfo,
) {
    let projects: Vec<Project> = match sqlx::query_as(
        "SELECT * FROM project
         WHERE repo_id = $1 AND repo_provider = $2 AND repo_base_url = $3
           AND status = 'active'",
    )
    .bind(repo_id)
    .bind(provider)
    .bind(base_url)
    .fetch_all(&state.db)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "{provider} webhook: project lookup failed");
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
    let changed = changed_paths(data);

    for project in projects {
        if !rules_allow(&project, &branch, &commit, &changed) {
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
                "{provider} push: deployment created"
            ),
            Err(e) => tracing::warn!(
                project_id = project.id,
                error = %e,
                "{provider} push: deployment create failed"
            ),
        }
    }
}

/// POST /api/bitbucket/webhook — `X-Event-Key: repo:push`. Bitbucket
/// doesn't sign payloads: `BITBUCKET_WEBHOOK_SECRET` (carried as
/// `?secret=` on the hook URL) is optional defense-in-depth, and the
/// claimed sha is ignored — the real branch head is resolved through
/// the project's connection before anything deploys.
pub async fn bitbucket_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    if let Some(secret) = &state.settings.bitbucket_webhook_secret {
        if q.get("secret") != Some(secret) {
            return Err(ApiError::unauthorized("invalid token"));
        }
    }
    let event = headers
        .get("x-event-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event != "repo:push" {
        return Ok(StatusCode::OK.into_response());
    }
    let data: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("invalid payload"))?;
    let full_name = data["repository"]["full_name"]
        .as_str()
        .map(String::from)
        // Older payloads: workspace.slug + repository.name.
        .or_else(|| {
            let ws = data["repository"]["workspace"]["slug"].as_str()?;
            let name = data["repository"]["name"].as_str()?;
            if ws.is_empty() || name.is_empty() {
                None
            } else {
                Some(format!("{ws}/{name}"))
            }
        })
        .unwrap_or_default();
    let branch = data["push"]["changes"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(|c| c["new"]["name"].as_str())
        .unwrap_or("")
        .to_string();
    if full_name.is_empty() || branch.is_empty() {
        return Ok(StatusCode::OK.into_response());
    }
    bitbucket_push(&state, &full_name, &branch).await;
    Ok(StatusCode::OK.into_response())
}

/// Bitbucket push — match projects on repo_full_name, resolve the real
/// branch head via the project's connection (never trust the webhook
/// sha), apply deployment rules, create + enqueue.
async fn bitbucket_push(state: &AppState, full_name: &str, branch: &str) {
    let projects: Vec<Project> = match sqlx::query_as(
        "SELECT * FROM project
         WHERE repo_provider = 'bitbucket' AND repo_full_name = $1 AND status = 'active'",
    )
    .bind(full_name)
    .fetch_all(&state.db)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "bitbucket webhook: project lookup failed");
            return;
        }
    };
    let Some((owner, repo)) = full_name.split_once('/') else {
        return;
    };
    for project in projects {
        let Some(conn_id) = project.bitbucket_connection_id else {
            continue;
        };
        let Ok(Some(conn)) =
            git_providers::connection(&state.db, &state.crypto, "bitbucket", conn_id).await
        else {
            continue;
        };
        let client = Client::new("bitbucket", conn);
        let head = match client.latest_commit(owner, repo, branch).await {
            Ok(Some(c)) => c,
            _ => continue,
        };
        let commit = CommitInfo {
            sha: head.sha,
            author: head.author,
            message: head.message,
            timestamp: head.timestamp,
        };
        // Bitbucket push payloads carry no per-commit file lists.
        if !rules_allow(&project, branch, &commit, &[]) {
            continue;
        }
        match deploy::create(
            &state.db,
            &state.bus,
            &state.crypto,
            &project,
            branch,
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
                "deployment created from bitbucket push"
            ),
            Err(e) => tracing::warn!(
                project_id = project.id,
                error = %e,
                "bitbucket push: deployment create failed"
            ),
        }
    }
}

/// Deployment-rules filter — same semantics as the github push handler.
/// Flatten `commits[].added|modified|removed` into a unique changed-path
/// list — the GitHub/Gitea/GitLab push-payload shape. Bitbucket's
/// `repo:push` carries no file lists, so it always passes `paths` rules.
pub(crate) fn changed_paths(data: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(commits) = data["commits"].as_array() {
        for c in commits {
            for key in ["added", "modified", "removed"] {
                if let Some(files) = c[key].as_array() {
                    out.extend(files.iter().filter_map(|f| f.as_str().map(String::from)));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub(crate) fn rules_allow(
    project: &Project,
    branch: &str,
    commit: &CommitInfo,
    changed: &[String],
) -> bool {
    let rules = project
        .config
        .get("deployment_rules")
        .cloned()
        .unwrap_or(json!({}));
    if rules.get("auto_deploy").and_then(|v| v.as_bool()) == Some(false) {
        return false;
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
        if !allowed.contains(&branch) {
            return false;
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
        return false;
    }
    if rules.get("skip_merge_commits").and_then(|v| v.as_bool()) == Some(true)
        && commit.message.starts_with("Merge")
    {
        return false;
    }
    runway_core::pathmatch::deployable(rules.get("paths").and_then(|v| v.as_str()), changed)
}
