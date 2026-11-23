//! Deployment routes: create (manual/latest commit), list, inspect,
//! cancel, skip, redeploy, rollback, promote, promotion diff,
//! logs (history + SSE stream).

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
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
        "project_name": project.name,
        "status": d.status,
        "computed_status": d.computed_status(),
        "conclusion": d.conclusion,
        "trigger": d.trigger,
        "environment_id": d.environment_id,
        "branch": d.branch,
        "commit_sha": d.commit_sha,
        "commit_meta": d.commit_meta,
        "repo_provider": d.repo_provider,
        "repo_full_name": d.repo_full_name,
        "repo_base_url": d.repo_base_url,
        "url": d.url(project.slug.as_deref().unwrap_or(&project.id), &state.settings),
        "urls": {
            "immutable": d.hostname(project.slug.as_deref().unwrap_or(&project.id), &state.settings),
            "environment": aliases.get("environment_domain"),
            "branch": aliases.get("branch_domain"),
        },
        "error": d.error,
        "container_id": d.container_id,
        "container_status": d.container_status,
        "observed_status": d.observed_status,
        "observed_exit_code": d.observed_exit_code,
        "observed_at": d.observed_at,
        "observed_missing_count": d.observed_missing_count,
        "created_at": d.created_at,
        "concluded_at": d.concluded_at,
    })
}

/// `GET /api/v1/deployments` — latest deployments across all projects the
/// user can see. Powers the global Deployments view + dashboard.
pub async fn index(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let projects = crate::routes::projects::user_projects(&state, user.user.id).await?;
    let ids: Vec<&str> = projects.iter().map(|p| p.id.as_str()).collect();
    let by_id: std::collections::HashMap<&str, &Project> =
        projects.iter().map(|p| (p.id.as_str(), p)).collect();
    let deployments: Vec<Deployment> = sqlx::query_as(
        "SELECT * FROM deployment WHERE project_id = ANY($1) \
         ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&ids)
    .fetch_all(&state.db)
    .await?;
    let items: Vec<Value> = deployments
        .iter()
        .filter_map(|d| {
            by_id.get(d.project_id.as_str()).map(|p| {
                let mut v = deployment_json(&state, d, p);
                v["project_name"] = json!(p.name);
                v
            })
        })
        .collect();
    Ok(Json(json!({ "deployments": items })).into_response())
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
    headers: axum::http::HeaderMap,
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

    let has_remote = match project.repo_provider.as_str() {
        "github" | "github_enterprise" => project.github_installation_id.is_some(),
        "gitea" | "forgejo" => project.gitea_connection_id.is_some(),
        "gitlab" => project.gitlab_connection_id.is_some(),
        "bitbucket" => project.bitbucket_connection_id.is_some(),
        _ => false,
    };

    let idem = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    let (dep, replayed) = if has_remote {
        trigger_deployment(&state, &project, &branch, "user", Some(user.user.id), idem).await?
    } else {
        // No git remote — redeploy the latest uploaded archive on this branch.
        let prev: Option<Deployment> = sqlx::query_as(
            "SELECT * FROM deployment WHERE project_id = $1 AND branch = $2 \
             AND config->>'source_archive' IS NOT NULL \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&project.id)
        .bind(&branch)
        .fetch_optional(&state.db)
        .await?;
        let Some(prev) = prev else {
            return Err(ApiError::bad_request(
                "project has no git remote — deploy via `runway deploy` first",
            ));
        };
        (
            redeploy_dep(&state, user.user.id, &prev, &project).await?,
            false,
        )
    };
    let mut a = runway_core::audit::Audit::new("deployment.create");
    a.user_id = Some(user.user.id);
    a.team_id = Some(project.team_id.clone());
    a.project_id = Some(project.id.clone());
    a.resource_type = Some("deployment");
    a.resource_id = Some(dep.id.clone());
    a.detail = Some(format!("branch={branch}"));
    runway_core::audit::log(&state.db, a).await;
    let mut body = deployment_json(&state, &dep, &project);
    if replayed {
        body["idempotent_replay"] = true.into();
    }
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// Shared: resolve latest commit via the repo's provider and
/// create+enqueue a deployment.
pub async fn trigger_deployment(
    state: &AppState,
    project: &Project,
    branch: &str,
    trigger: &str,
    user_id: Option<i64>,
    idempotency_key: Option<&str>,
) -> ApiResult<(Deployment, bool)> {
    // `Idempotency-Key` — replay the existing deployment instead of
    // queueing a duplicate. jarvis: check-then-create has a small race
    // window; upgrade to a partial unique index if retries get noisy.
    let key = idempotency_key.map(str::trim).filter(|k| !k.is_empty());
    if let Some(key) = key {
        let existing: Option<Deployment> = sqlx::query_as(
            "SELECT * FROM deployment
             WHERE project_id = $1 AND config->>'idempotency_key' = $2
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&project.id)
        .bind(key)
        .fetch_optional(&state.db)
        .await?;
        if let Some(d) = existing {
            return Ok((d, true));
        }
    }

    let info = deploy::resolve_commit(
        &state.db,
        &state.crypto,
        state.github.if_configured(),
        project,
        branch,
    )
    .await
    .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if info.sha.is_empty() {
        return Err(ApiError::bad_request("could not resolve commit"));
    }

    let extra = key.map(|k| json!({ "idempotency_key": k }));
    Ok((
        deploy::create(
            &state.db,
            &state.bus,
            &state.crypto,
            project,
            branch,
            &info,
            trigger,
            user_id,
            extra,
        )
        .await?,
        false,
    ))
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let mut body = deployment_json(&state, &dep, &project);
    // Alias state for this deployment's environment — lets the UI mark the
    // live deployment and gate "instant rollback" on the actual target.
    body["env_state"] =
        match deploy::environment_alias(&state.db, &project, &dep.environment_id).await? {
            Some(a) => json!({
                "subdomain": a.subdomain,
                "current_deployment_id": a.deployment_id,
                "previous_deployment_id": a.previous_deployment_id,
            }),
            None => Value::Null,
        };
    // Production alias state as well — gates "Promote to production".
    body["prod_state"] = if dep.environment_id == "prod" {
        body["env_state"].clone()
    } else {
        match deploy::environment_alias(&state.db, &project, "prod").await? {
            Some(a) => json!({
                "subdomain": a.subdomain,
                "current_deployment_id": a.deployment_id,
                "previous_deployment_id": a.previous_deployment_id,
            }),
            None => Value::Null,
        }
    };
    Ok(Json(body).into_response())
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

/// Shared redeploy: same branch + commit + source archive as `dep`.
pub(crate) async fn redeploy_dep(
    state: &AppState,
    user_id: i64,
    dep: &Deployment,
    project: &Project,
) -> ApiResult<Deployment> {
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
    let extra = dep
        .config
        .get("source_archive")
        .cloned()
        .map(|a| json!({ "source_archive": a }));
    if let Some(archive) = dep.config.get("source_archive").and_then(|v| v.as_str()) {
        let safe = archive
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        let exists = safe
            && std::path::Path::new(&state.settings.data_dir)
                .join("uploads")
                .join(archive)
                .exists();
        if !exists {
            return Err(ApiError::bad_request(
                "uploaded source no longer retained — deploy again via `runway deploy`",
            ));
        }
    }
    Ok(deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        project,
        &dep.branch,
        &info,
        "user",
        Some(user_id),
        extra,
    )
    .await?)
}

/// Redeploy: same branch + same commit, fresh deployment.
pub async fn redeploy(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let new_dep = redeploy_dep(&state, user.user.id, &dep, &project).await?;
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

#[derive(Deserialize)]
pub struct PromoteBody {
    /// Target environment — defaults to production (Vercel's "Promote").
    pub environment_id: Option<String>,
}

/// Promote a deployment so an environment's alias serves it.
pub async fn promote(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<PromoteBody>>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let environment_id = body
        .and_then(|b| b.0.environment_id)
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "prod".into());
    let alias = deploy::promote(
        &state.db,
        &state.bus,
        &state.settings,
        &project,
        &environment_id,
        &dep,
    )
    .await?;
    Ok(Json(json!({
        "subdomain": alias.subdomain,
        "environment_id": environment_id,
        "deployment_id": alias.deployment_id,
        "previous_deployment_id": alias.previous_deployment_id,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct DiffQuery {
    pub environment_id: Option<String>,
}

/// `GET /api/v1/deployments/{id}/diff?environment_id=prod` — key-level
/// preview of a promotion. Env values stay on the server.
pub async fn promotion_diff(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let environment_id = q
        .environment_id
        .filter(|env| !env.is_empty())
        .unwrap_or_else(|| "prod".into());
    let diff = runway_core::promote_diff::preview(
        &state.db,
        &state.crypto,
        &state.settings,
        &project,
        &dep,
        &environment_id,
    )
    .await?;
    Ok(Json(diff).into_response())
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

/// `GET /api/v1/deployments/{id}/og?path=/` — fetch the deployed page
/// server-side and report its Open Graph / Twitter meta. Always 200: a
/// failed fetch returns `{ok:false}` so the UI can show a calm state.
#[derive(Deserialize)]
pub struct OgQuery {
    pub path: Option<String>,
}

pub async fn open_graph(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<OgQuery>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    let slug = project.slug.as_deref().unwrap_or(&project.id);
    // Path must be a bare path — strip any scheme/authority attempts.
    let path = q.path.as_deref().unwrap_or("/").trim();
    let path = if path.starts_with('/') { path } else { "/" };
    let url = format!("{}{}", dep.url(slug, &state.settings), path);
    let result = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(ApiError::internal)?
        .get(&url)
        .send()
        .await;
    match result {
        Ok(resp) if resp.status().is_success() => {
            let bytes = resp.bytes().await.map_err(ApiError::internal)?;
            let html = String::from_utf8_lossy(&bytes[..bytes.len().min(256 * 1024)]);
            let report = runway_core::opengraph::parse(&html);
            Ok(Json(json!({ "ok": true, "url": url, "report": report })).into_response())
        }
        Ok(resp) => Ok(Json(
            json!({ "ok": false, "url": url, "reason": format!("HTTP {}", resp.status()) }),
        )
        .into_response()),
        Err(e) => {
            Ok(Json(json!({ "ok": false, "url": url, "reason": e.to_string() })).into_response())
        }
    }
}

/// `GET /api/v1/deployments/{id}/files` — inventory of the deployed
/// static output (`data/static/<id>/`), Vercel's Resources tab.
/// Always 200: `reason` distinguishes a static deploy whose published
/// artifact is gone (`static_output_missing`) from a server/image deploy
/// that never had a file payload (`no_file_payload`).
pub async fn files(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, _p) = accessible_deployment(&state, user.user.id, &id).await?;
    let dir = std::path::Path::new(&state.settings.data_dir)
        .join("static")
        .join(&dep.id);
    let files = tokio::task::spawn_blocking(move || walk_files(&dir))
        .await
        .map_err(ApiError::internal)?;
    match files {
        Some(list) => Ok(Json(json!({
            "ok": true,
            "files": list
                .iter()
                .map(|(p, s)| json!({ "path": p, "size": s }))
                .collect::<Vec<_>>(),
        }))
        .into_response()),
        None => {
            let reason = if dep.is_static() {
                "static_output_missing"
            } else {
                "no_file_payload"
            };
            Ok(Json(json!({ "ok": false, "reason": reason })).into_response())
        }
    }
}

/// Recursive file listing under `dir` as (relative path, size) pairs —
/// None when the directory doesn't exist.
fn walk_files(dir: &std::path::Path) -> Option<Vec<(String, u64)>> {
    if !dir.is_dir() {
        return None;
    }
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                let rel = path.strip_prefix(dir).unwrap_or(&path);
                out.push((rel.to_string_lossy().replace('\\', "/"), meta.len()));
            }
        }
    }
    out.sort();
    Some(out)
}

/// `GET /api/v1/projects/{id}/logs` — merged tail across the project's
/// recent deployments (devpush `project_logs`, file-backed instead of
/// Loki). Output lines: `<rfc3339>\t<dep7>\t<text>`, sorted by time.
pub async fn project_logs(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let tail = q.tail.unwrap_or(200).clamp(1, 5000);
    let dep_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM deployment WHERE project_id = $1 \
         ORDER BY created_at DESC LIMIT 8",
    )
    .bind(&project.id)
    .fetch_all(&state.db)
    .await?;
    // (timestamp prefix, short id, text) — RFC3339 sorts lexically.
    let mut merged: Vec<(String, String, String)> = Vec::new();
    for id in dep_ids {
        let short = id[..7.min(id.len())].to_string();
        for line in state.logs.tail(&id, tail).await.unwrap_or_default() {
            let mut parts = line.splitn(3, '\t');
            let ts = parts.next().unwrap_or_default().to_string();
            let _stream = parts.next();
            let text = parts.next().unwrap_or_default().to_string();
            merged.push((ts, short.clone(), text));
        }
    }
    merged.sort();
    let body = merged
        .iter()
        .rev()
        .take(tail)
        .rev()
        .map(|(ts, dep, text)| format!("{ts}\t{dep}\t{text}"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(axum::response::Response::builder()
        .header("content-type", "text/plain; charset=utf-8")
        .body(axum::body::Body::from(body))
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

/// One-shot container resource stats: cpu %, memory bytes, network io,
/// pids. 404-shape `{running:false}` when the container isn't live.
pub async fn stats(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, _p) = accessible_deployment(&state, user.user.id, &id).await?;
    let Some(cid) = dep.container_id.clone() else {
        return Ok(Json(json!({ "running": false })).into_response());
    };
    let docker = if let Some(nid) = dep.remote_node_id.as_deref() {
        match runway_core::docker::node_client(&state.db, nid, &state.crypto).await {
            Some(c) => c,
            None => return Ok(Json(json!({ "running": false })).into_response()),
        }
    } else {
        match state.docker.clone() {
            Some(d) => d,
            None => return Ok(Json(json!({ "running": false })).into_response()),
        }
    };
    let Some(s) = runway_core::docker::stats_snapshot(&docker, &cid).await else {
        return Ok(Json(json!({ "running": false })).into_response());
    };
    Ok(Json(json!({
        "running": true,
        "cpu_pct": (s.cpu_pct * 100.0).round() / 100.0,
        "mem_used": s.mem_used,
        "mem_limit": s.mem_limit,
        "net_rx": s.net_rx,
        "net_tx": s.net_tx,
        "pids": s.pids,
    }))
    .into_response())
}

/// `POST /api/v1/deployments/{id}/reconcile` — probe the container now
/// instead of waiting for the next monitor tick; returns the fresh
/// deployment row. Port of project_deployment_reconcile.
pub async fn reconcile(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, project) = accessible_deployment(&state, user.user.id, &id).await?;
    if let Some(cid) = dep.container_id.clone() {
        let docker = if let Some(nid) = dep.remote_node_id.as_deref() {
            runway_core::docker::node_client(&state.db, nid, &state.crypto).await
        } else {
            state.docker.clone()
        };
        if let Some(docker) = docker {
            if let Some((observed, exit_code)) =
                runway_core::docker::inspect_observed(&docker, &cid).await
            {
                let missing = if observed == "not_found" {
                    dep.observed_missing_count + 1
                } else {
                    0
                };
                sqlx::query(
                    "UPDATE deployment SET observed_status = $1, observed_exit_code = $2,
                     observed_at = now(), observed_last_seen_at = now(),
                     observed_missing_count = $3 WHERE id = $4",
                )
                .bind(&observed)
                .bind(exit_code)
                .bind(missing)
                .bind(&dep.id)
                .execute(&state.db)
                .await?;
            }
        }
    }
    let dep = deploy::get(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("deployment"))?;
    Ok(Json(deployment_json(&state, &dep, &project)).into_response())
}

/// `GET /api/v1/deployments/{id}/metrics` — last 24h of monitor samples
/// (30s cadence while the container runs) for sparklines.
pub async fn metrics(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (dep, _p) = accessible_deployment(&state, user.user.id, &id).await?;
    let rows: Vec<(chrono::DateTime<chrono::Utc>, f64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT ts, cpu_pct, mem_used, net_rx, net_tx, pids FROM deployment_metric
         WHERE deployment_id = $1 AND ts > now() - interval '24 hours'
         ORDER BY ts ASC",
    )
    .bind(&dep.id)
    .fetch_all(&state.db)
    .await?;
    let samples: Vec<Value> = rows
        .iter()
        .map(|(ts, cpu, mem, rx, tx, pids)| {
            json!({
                "ts": ts, "cpu_pct": cpu, "mem_used": mem,
                "net_rx": rx, "net_tx": tx, "pids": pids,
            })
        })
        .collect();
    Ok(Json(json!({ "samples": samples })).into_response())
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
    Query(q): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> ApiResult<Response> {
    const MAX_UPLOAD: u64 = 512 * 1024 * 1024; // jarvis: 512MB ceiling; revisit if monorepos hurt

    let project = accessible_project(&state, user.user.id, &id).await?;
    let dry_run = q
        .get("dry_run")
        .is_some_and(|v| matches!(v.as_str(), "1" | "true"));

    // Fail fast before streaming up to 512MB when an in-flight deployment
    // already holds this project's target environment. If the client sent
    // X-Runway-Upload-Sha matching the in-flight deployment (retry after an
    // edge 504), return that deployment idempotently so the CLI can attach.
    if !dry_run {
        if let Some(existing) = deploy::in_flight(&state.db, &project, &project.repo_branch).await?
        {
            let req_sha = headers
                .get("x-runway-upload-sha")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let existing_sha = existing.commit_meta.get("sha").and_then(|v| v.as_str());
            if req_sha.is_some() && req_sha == existing_sha {
                let mut payload = deployment_json(&state, &existing, &project);
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("resumed".into(), json!(true));
                }
                return Ok((StatusCode::OK, Json(payload)).into_response());
            }
            let env_slug = project
                .environment_for_deploy(&project.repo_branch)
                .map(|e| e.env_scope_slug().to_string())
                .unwrap_or_else(|| "production".into());
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "deployment {} is already in progress for environment '{env_slug}'; \
                         attach with `runway logs {} --follow` or cancel it (`runway cancel {}`) and retry",
                        existing.id, existing.id, existing.id
                    ),
                    "code": "deployment_in_progress",
                    "deployment_id": existing.id,
                })),
            )
                .into_response());
        }
    }

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

    // vercel.json → runway.json import mutates the archive in place, so it
    // runs before detection and both detection + dry-run see the result.
    let vercel_import = import_vercel_json(&path).await?;

    // Git providers detect the framework at project creation; upload
    // projects have no source then — detect from this archive instead and
    // stamp the result onto the deployment's config snapshot. Explicit
    // runner fields in project config always win.
    let mut extra = json!({ "source_archive": format!("{dep_id}.tar.gz") });
    let has_runner = ["runner", "image", "override_image", "dockerfile_path"]
        .iter()
        .any(|k| project.config.get(k).is_some());
    if !has_runner {
        if let Some(detected) = detect_upload_preset(&path).await? {
            if let (Some(dst), Some(src)) = (extra.as_object_mut(), detected.as_object()) {
                for (k, v) in src {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // `?dry_run=1` — report the resolved plan without creating a
    // deployment. Agents use this to preflight what an upload would do.
    if dry_run {
        let _ = tokio::fs::remove_file(&path).await;
        let mut plan = project.config.clone();
        if let (Some(dst), Some(src)) = (plan.as_object_mut(), extra.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        return Ok(Json(json!({
            "dry_run": true,
            "size_bytes": size,
            "resolved_config": plan,
            "vercel_import": vercel_import,
        }))
        .into_response());
    }

    let sha = hex::encode(hasher.finalize())[..40].to_string();
    let branch = project.repo_branch.clone();
    let commit = CommitInfo {
        sha,
        message: "Uploaded via CLI".into(),
        author: user.user.username.clone(),
        timestamp: None,
    };
    let dep = match deploy::create(
        &state.db,
        &state.bus,
        &state.crypto,
        &project,
        &branch,
        &commit,
        "api",
        Some(user.user.id),
        Some(extra),
    )
    .await
    {
        Ok(dep) => dep,
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e.into());
        }
    };
    Ok((
        StatusCode::CREATED,
        Json(deployment_json(&state, &dep, &project)),
    )
        .into_response())
}

/// Inspect an uploaded tarball for a runnable framework. Returns the
/// preset fields (or `dockerfile_path`) to stamp into the deployment
/// config, `None` when nothing matches. A corrupt archive is rejected
/// here — the pipeline's in-container extract would only fail later.
async fn detect_upload_preset(path: &std::path::Path) -> ApiResult<Option<Value>> {
    let listing = tokio::process::Command::new("tar")
        .args(["-tzf"])
        .arg(path)
        .output()
        .await
        .map_err(ApiError::internal)?;
    if !listing.status.success() {
        return Err(ApiError::bad_request("invalid gzip archive"));
    }
    let raw_lines: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    let files: Vec<String> = raw_lines
        .iter()
        .map(|l| l.trim_start_matches("./").to_string())
        .filter(|l| !l.is_empty() && !l.ends_with('/'))
        .collect();
    // Package manager + framework checks mirror creation-time detection:
    // root-level entries only (monorepo root_dir is a deployment concern).
    let root: Vec<String> = files.iter().filter(|f| !f.contains('/')).cloned().collect();

    // `tar -x` matches member names verbatim — a `./package.json` archive
    // (GNU tar's default when archiving `.`) is not `package.json`.
    let package_json = raw_lines
        .iter()
        .find(|l| l.trim_start_matches("./") == "package.json")
        .cloned();
    let package_json = if let Some(member) = package_json {
        let out = tokio::process::Command::new("tar")
            .args(["-xzOf"])
            .arg(path)
            .arg(&member)
            .output()
            .await
            .map_err(ApiError::internal)?;
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    };

    let mut config = json!({});
    let refs: Vec<&str> = root.iter().map(String::as_str).collect();
    if let Some(preset) = runway_core::presets::detect(&refs, package_json.as_deref()) {
        crate::routes::projects::apply_preset(&mut config, preset);
        runway_core::presets::adjust_for_pm(
            &mut config,
            runway_core::presets::package_manager(&refs),
        );
        return Ok(Some(config));
    }
    if root.iter().any(|f| f == "Dockerfile") {
        config["dockerfile_path"] = "Dockerfile".into();
        return Ok(Some(config));
    }
    Ok(None)
}

/// `vercel.json` maps onto the runway.json contract — when an upload
/// carries one at archive root (and no runway.json), synthesize a
/// runway.json into the tarball so in-container `readcfg` and the SWS
/// config path consume it unchanged. Returns the translated document.
async fn import_vercel_json(path: &std::path::Path) -> ApiResult<Option<Value>> {
    let listing = tokio::process::Command::new("tar")
        .args(["-tzf"])
        .arg(path)
        .output()
        .await
        .map_err(ApiError::internal)?;
    if !listing.status.success() {
        return Ok(None); // corrupt archives are rejected by detect_upload_preset
    }
    let raw_lines: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    let root_member = |name: &str| {
        raw_lines
            .iter()
            .find(|l| l.trim_start_matches("./") == name)
            .cloned()
    };
    if root_member("runway.json").is_some() {
        return Ok(None); // explicit runway.json wins
    }
    let Some(member) = root_member("vercel.json") else {
        return Ok(None);
    };

    let out = tokio::process::Command::new("tar")
        .args(["-xzOf"])
        .arg(path)
        .arg(&member)
        .output()
        .await
        .map_err(ApiError::internal)?;
    let vj: Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| ApiError::bad_request("invalid vercel.json"))?;

    // Translatable keys only; vercel-only fields (framework, devCommand,
    // regions, functions) are dropped — preset detection covers the rest.
    let mut rj = serde_json::Map::new();
    fn str_key<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
        v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty())
    }
    let cmd = [str_key(&vj, "installCommand"), str_key(&vj, "buildCommand")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" && ");
    if !cmd.is_empty() {
        rj.insert("build_command".into(), cmd.into());
    }
    if let Some(o) = str_key(&vj, "outputDirectory") {
        rj.insert("output_directory".into(), o.into());
    }
    for k in ["rewrites", "redirects", "headers"] {
        if let Some(v) = vj.get(k) {
            rj.insert(k.into(), v.clone());
        }
    }
    if rj.is_empty() {
        return Ok(None);
    }

    // Repack: extract, add runway.json, recompress atomically.
    let dir = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(format!(
            "{}.import",
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("upload")
        ));
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(ApiError::internal)?;
    let cleanup = |dir: &std::path::Path| {
        let dir = dir.to_path_buf();
        async move {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    };
    let status = tokio::process::Command::new("tar")
        .args(["-xzf"])
        .arg(path)
        .arg("-C")
        .arg(&dir)
        .status()
        .await
        .map_err(ApiError::internal)?;
    if !status.success() {
        cleanup(&dir).await;
        return Err(ApiError::bad_request("vercel.json import: extract failed"));
    }
    tokio::fs::write(
        dir.join("runway.json"),
        serde_json::to_vec_pretty(&Value::Object(rj.clone())).map_err(ApiError::internal)?,
    )
    .await
    .map_err(ApiError::internal)?;
    let tmp_out = dir.with_extension("tgz");
    let status = tokio::process::Command::new("tar")
        .args(["-czf"])
        .arg(&tmp_out)
        .arg("-C")
        .arg(&dir)
        .arg(".")
        .status()
        .await
        .map_err(ApiError::internal)?;
    cleanup(&dir).await;
    if !status.success() {
        let _ = tokio::fs::remove_file(&tmp_out).await;
        return Err(ApiError::bad_request("vercel.json import: repack failed"));
    }
    tokio::fs::rename(&tmp_out, path)
        .await
        .map_err(ApiError::internal)?;
    Ok(Some(Value::Object(rj)))
}
