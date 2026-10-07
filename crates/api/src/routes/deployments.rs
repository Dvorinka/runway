//! Deployment routes: create (manual/latest commit), list, inspect,
//! cancel, skip, redeploy, rollback, logs (history + SSE stream).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::deploy::{self, CommitInfo};
use runway_core::models::{Deployment, Project};

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

async fn accessible_project(state: &AppState, user_id: i64, id: &str) -> ApiResult<Project> {
    let project: Option<Project> = sqlx::query_as(
        "SELECT p.* FROM project p
         JOIN team_member tm ON tm.team_id = p.team_id
         WHERE tm.user_id = $1 AND p.id = $2 AND p.status != 'deleted'",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    project.ok_or_else(|| ApiError::not_found("project"))
}

async fn accessible_deployment(
    state: &AppState,
    user_id: i64,
    id: &str,
) -> ApiResult<(Deployment, Project)> {
    let dep = deploy::get(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("deployment"))?;
    let project = accessible_project(state, user_id, &dep.project_id).await?;
    Ok((dep, project))
}

fn deployment_json(state: &AppState, d: &Deployment, project: &Project) -> Value {
    let aliases = deploy::alias_domains(d, project, &state.settings);
    json!({
        "id": d.id,
        "project_id": d.project_id,
        "status": d.status,
        "computed_status": d.computed_status(),
        "conclusion": d.conclusion,
        "trigger": d.trigger,
        "environment_id": d.environment_id,
        "branch": d.branch,
        "commit_sha": d.commit_sha,
        "commit_meta": d.commit_meta,
        "url": d.url(project.slug.as_deref().unwrap_or(&project.id), &state.settings),
        "urls": {
            "immutable": d.hostname(project.slug.as_deref().unwrap_or(&project.id), &state.settings),
            "environment": aliases.get("environment_domain"),
            "branch": aliases.get("branch_domain"),
        },
        "error": d.error,
        "container_id": d.container_id,
        "container_status": d.container_status,
        "created_at": d.created_at,
        "concluded_at": d.concluded_at,
    })
}

pub async fn list(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let limit = q.limit.unwrap_or(30).clamp(1, 100);
    let deployments: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&project.id)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let items: Vec<Value> = deployments
        .iter()
        .map(|d| deployment_json(&state, d, &project))
        .collect();
    Ok(Json(json!({ "deployments": items })).into_response())
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateDeployment {
    /// Branch to deploy (defaults to the production branch).
    pub branch: Option<String>,
}

/// Trigger a deployment on the latest commit of a branch.
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    body: Option<Json<CreateDeployment>>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let branch = body
        .and_then(|b| b.0.branch)
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| {
            if project.repo_branch.is_empty() {
                "main".into()
            } else {
                project.repo_branch.clone()
            }
        });

    let dep = trigger_deployment(&state, &project, &branch, "user", Some(user.user.id)).await?;
    Ok((
        StatusCode::CREATED,
        Json(deployment_json(&state, &dep, &project)),
    )
        .into_response())
}

/// Shared: resolve latest commit via GitHub and create+enqueue a deployment.
pub async fn trigger_deployment(
    state: &AppState,
    project: &Project,
    branch: &str,
    trigger: &str,
    user_id: Option<i64>,
) -> ApiResult<Deployment> {
    let Some(gh) = &state.github else {
        return Err(ApiError::bad_request("GitHub App is not configured"));
    };
    let installation_id = project
        .github_installation_id
        .ok_or_else(|| ApiError::bad_request("project has no GitHub installation"))?;
    let token = gh
        .installation_token(&state.db, &state.crypto, installation_id)
        .await
        .map_err(ApiError::internal)?;
    let commit = gh
        .latest_commit(&token, &project.repo_full_name, branch)
        .await
        .map_err(|_| ApiError::bad_request(format!("branch '{branch}' not found on remote")))?;

    let info = CommitInfo {
        sha: commit["sha"].as_str().unwrap_or_default().to_string(),
        message: commit["commit"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        author: commit["commit"]["author"]["name"]
            .as_str()
            .or_else(|| commit["author"]["login"].as_str())
            .unwrap_or_default()
            .to_string(),
        timestamp: commit["commit"]["author"]["date"]
            .as_str()
            .map(String::from),
    };
    if info.sha.is_empty() {
        return Err(ApiError::bad_request("could not resolve commit"));
    }

    Ok(deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        project,
        branch,
        &info,
        trigger,
        user_id,
        None,
    )
    .await?)
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    Ok(Json(deployment_json(&state, &dep, &project)).into_response())
}

pub async fn cancel(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let docker = runway_core::docker::connect(&state.settings).map_err(ApiError::from)?;
    deploy::cancel(&state.db, &state.bus, &state.settings, &docker, &dep).await?;
    let dep = deploy::get(&state.db, &id).await?.unwrap();
    runway_core::webhook::send_deployment_webhooks(
        &state.db,
        &state.crypto,
        &state.settings,
        &project,
        &dep,
        "canceled",
    )
    .await;
    Ok(
        Json(json!({ "id": dep.id, "status": dep.status, "conclusion": dep.conclusion }))
            .into_response(),
    )
}

pub async fn skip(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    deploy::skip(&state.db, &state.bus, &dep).await?;
    if let Some(dep) = deploy::get(&state.db, &id).await? {
        runway_core::webhook::send_deployment_webhooks(
            &state.db,
            &state.crypto,
            &state.settings,
            &project,
            &dep,
            "skipped",
        )
        .await;
    }
    Ok(Json(json!({ "ok": true })).into_response())
}

/// Redeploy: same branch + same commit, fresh deployment.
pub async fn redeploy(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let info = CommitInfo {
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
        timestamp: dep
            .commit_meta
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(String::from),
    };
    let new_dep = deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        &project,
        &dep.branch,
        &info,
        "user",
        Some(user.user.id),
        None,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(deployment_json(&state, &new_dep, &project)),
    )
        .into_response())
}

/// Rollback an environment to its previous deployment.
pub async fn rollback(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, environment_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let alias = deploy::rollback(
        &state.db,
        &state.bus,
        &state.settings,
        &project,
        &environment_id,
    )
    .await?;
    Ok(Json(json!({
        "subdomain": alias.subdomain,
        "deployment_id": alias.deployment_id,
        "previous_deployment_id": alias.previous_deployment_id,
    }))
    .into_response())
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LogsQuery {
    pub tail: Option<usize>,
}

/// Historical log tail (plain text lines).
pub async fn logs(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Response> {
    let (_dep, _p) = accessible_deployment(&state, user.user.id, &id).await?;
    let lines = state
        .logs
        .tail(&id, q.tail.unwrap_or(200).min(5000))
        .await
        .map_err(ApiError::from)?;
    Ok(axum::response::Response::builder()
        .header("content-type", "text/plain; charset=utf-8")
        .body(axum::body::Body::from(lines.join("\n")))
        .unwrap()
        .into_response())
}

/// SSE stream: replays the log file, then follows live lines.
pub async fn logs_stream(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (_dep, _p) = accessible_deployment(&state, user.user.id, &id).await?;
    let history = state.logs.tail(&id, 1000).await.unwrap_or_default();
    let rx = state.bus.subscribe(&runway_core::logs::scope(&id));

    let history_stream = futures::stream::iter(
        history
            .into_iter()
            .map(|line| Ok::<_, std::convert::Infallible>(Event::default().data(line))),
    );
    let live = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|item| async move {
        item.ok()
            .map(|line| Ok::<_, std::convert::Infallible>(Event::default().data(line)))
    });
    let stream = history_stream.chain(live);
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// SSE stream of project-level deployment events.
pub async fn events(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let _project = accessible_project(&state, user.user.id, &project_id).await?;
    let rx = state
        .bus
        .subscribe(&format!("project:{project_id}:updates"));
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|item| async move {
        item.ok()
            .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)))
    });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `POST /api/v1/projects/{id}/deployments/upload` — deploy a local tarball
/// (gzip) without git. Streams the body to `data/uploads/<id>.tar.gz`; the
/// pipeline extracts it instead of cloning. Port of devpush's upload-deploy
/// flow, minus multipart (raw `application/gzip` body keeps CLI trivial).
pub async fn upload(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Body,
) -> ApiResult<Response> {
    const MAX_UPLOAD: u64 = 512 * 1024 * 1024; // jarvis: 512MB ceiling; revisit if monorepos hurt

    let project = accessible_project(&state, user.user.id, &id).await?;

    let dep_id = runway_core::slugify::token_hex(16);
    let dir = std::path::Path::new(&state.settings.data_dir).join("uploads");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(ApiError::internal)?;
    let path = dir.join(format!("{dep_id}.tar.gz"));

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(ApiError::internal)?;
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut size: u64 = 0;
    let mut stream = body.into_data_stream();
    let result: ApiResult<()> = async {
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(ApiError::internal)?;
            size += chunk.len() as u64;
            if size > MAX_UPLOAD {
                return Err(ApiError::bad_request("upload exceeds 512MB limit"));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
        file.flush().await.map_err(ApiError::internal)?;
        if size == 0 {
            return Err(ApiError::bad_request("empty upload body"));
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }

    let sha = hex::encode(hasher.finalize())[..40].to_string();
    let branch = project.repo_branch.clone();
    let commit = CommitInfo {
        sha,
        message: "Uploaded via CLI".into(),
        author: user.user.username.clone(),
        timestamp: None,
    };
    let dep = deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        &project,
        &branch,
        &commit,
        "api",
        Some(user.user.id),
        Some(json!({ "source_archive": format!("{dep_id}.tar.gz") })),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(deployment_json(&state, &dep, &project)),
    )
        .into_response())
}
