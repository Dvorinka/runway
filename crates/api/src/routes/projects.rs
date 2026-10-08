//! Project routes: create (from a GitHub repo), list, inspect, env vars,
//! deploy tokens, domains.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::models::{DeployToken, Deployment, Domain, Environment, Project};
use runway_core::slugify::token_hex;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) async fn user_projects(state: &AppState, user_id: i64) -> ApiResult<Vec<Project>> {
    Ok(sqlx::query_as::<_, Project>(
        "SELECT p.* FROM project p
         JOIN team_member tm ON tm.team_id = p.team_id
         WHERE tm.user_id = $1 AND p.status = 'active'
         ORDER BY p.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?)
}

/// Load a project the user can access (membership on its team).
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

async fn default_team_id(state: &AppState, user: &runway_core::models::User) -> ApiResult<String> {
    if let Some(id) = &user.default_team_id {
        return Ok(id.clone());
    }
    let team: Option<(String,)> =
        sqlx::query_as("SELECT team_id FROM team_member WHERE user_id = $1 ORDER BY id LIMIT 1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    team.map(|t| t.0)
        .ok_or_else(|| ApiError::bad_request("user has no team"))
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

pub async fn list(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let projects = user_projects(&state, user.user.id).await?;
    let latest: Vec<Deployment> = sqlx::query_as(
        "SELECT DISTINCT ON (project_id) * FROM deployment \
         ORDER BY project_id, created_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    let latest_by_project: std::collections::HashMap<&str, &Deployment> =
        latest.iter().map(|d| (d.project_id.as_str(), d)).collect();
    let items: Vec<Value> = projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "slug": p.slug,
                "url": p.url(&state.settings),
                "repo_provider": p.repo_provider,
                "repo_full_name": p.repo_full_name,
                "repo_branch": p.repo_branch,
                "repo_status": p.repo_status,
                "preset": p.config.get("preset"),
                "environments": p.environments,
                "created_at": p.created_at,
                "latest_deployment": latest_by_project.get(p.id.as_str()).map(|d| json!({
                    "id": d.id,
                    "status": d.status,
                    "computed_status": d.computed_status(),
                    "conclusion": d.conclusion,
                    "commit_sha": d.commit_sha,
                    "commit_meta": d.commit_meta,
                    "branch": d.branch,
                    "environment_id": d.environment_id,
                    "created_at": d.created_at,
                    "concluded_at": d.concluded_at,
                })),
            })
        })
        .collect();
    Ok(Json(json!({ "projects": items })).into_response())
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub name: String,
    /// Provider: `github` (default) | `github_enterprise` | `gitea` |
    /// `gitlab` | `bitbucket`.
    pub provider: Option<String>,
    /// GitHub repo numeric id (github providers). Gitea/GitLab resolve
    /// the id from `repo_full_name` via the connection.
    pub repo_id: Option<i64>,
    /// e.g. "acme/site".
    pub repo_full_name: String,
    /// GitHub installation id (github providers only).
    pub installation_id: Option<i64>,
    /// gitea/gitlab connection id (from `POST /api/v1/git/{p}/connect`).
    pub connection_id: Option<i64>,
    /// Production branch (default "main").
    pub branch: Option<String>,
    /// Preset slug (e.g. "nextjs"); merged into config.
    pub preset: Option<String>,
    /// Extra config map (build/start commands, port, root_directory...).
    pub config: Option<Value>,
}

pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateProject>,
) -> ApiResult<Response> {
    let provider = body.provider.as_deref().unwrap_or("github");
    let team_id = default_team_id(&state, &user.user).await?;
    let branch = body.branch.clone().unwrap_or_else(|| "main".into());

    // Resolve the runner image up front so bad presets fail fast.
    let mut config = body.config.unwrap_or_else(|| json!({}));
    let apply_preset = |config: &mut Value, preset: &'static runway_core::presets::Preset| {
        let obj = config.as_object_mut().unwrap();
        obj.entry("runner".to_string())
            .or_insert_with(|| preset.runner.into());
        obj.entry("build_command".to_string())
            .or_insert_with(|| preset.build_command.into());
        obj.entry("start_command".to_string())
            .or_insert_with(|| preset.start_command.into());
        obj.entry("port".to_string())
            .or_insert_with(|| preset.port.into());
        if let Some(output_dir) = preset.output_dir {
            obj.entry("output_directory".to_string())
                .or_insert_with(|| output_dir.into());
            obj.entry("spa_fallback".to_string())
                .or_insert_with(|| preset.spa_fallback.into());
        }
        obj.entry("preset".to_string())
            .or_insert_with(|| preset.slug.into());
    };
    if let Some(preset_slug) = &body.preset {
        let preset = runway_core::presets::preset(preset_slug)
            .ok_or_else(|| ApiError::bad_request("unknown preset"))?;
        apply_preset(&mut config, preset);
    }

    // Verify repo access + resolve repo_id/base_url per provider.
    let mut repo_id = body.repo_id.unwrap_or(0);
    let mut repo_base_url = "https://github.com".to_string();
    let mut gitea_connection_id: Option<i64> = None;
    let mut gitlab_connection_id: Option<i64> = None;
    let mut bitbucket_connection_id: Option<i64> = None;
    let mut root_files: Vec<String> = Vec::new();
    let mut package_json: Option<String> = None;

    match provider {
        "github" | "github_enterprise" => {
            let gh = state
                .github
                .if_configured()
                .ok_or_else(|| ApiError::bad_request("GitHub App is not configured"))?;
            let installation_id = body.installation_id.ok_or_else(|| {
                ApiError::bad_request("installation_id required for github projects")
            })?;
            let token = gh
                .installation_token(&state.db, &state.crypto, installation_id)
                .await
                .map_err(|e| ApiError::bad_request(format!("installation token failed: {e}")))?;
            let repo = gh
                .repository(&token, repo_id)
                .await
                .map_err(|_| ApiError::bad_request("repository not accessible via installation"))?;
            if provider == "github_enterprise" {
                repo_base_url = state
                    .settings
                    .github_api_url
                    .trim_end_matches("/api/v3")
                    .to_string();
                let _ = repo;
            }
            if body.preset.is_none() && config.get("runner").is_none() {
                if let Ok(files) = gh.repo_files(&token, &body.repo_full_name, &branch).await {
                    root_files = files;
                    if root_files.iter().any(|f| f == "package.json") {
                        package_json = gh
                            .file_text(&token, &body.repo_full_name, "package.json")
                            .await
                            .ok()
                            .flatten();
                    }
                }
            }
        }
        p @ ("gitea" | "gitlab" | "bitbucket") => {
            let conn_id = body
                .connection_id
                .ok_or_else(|| ApiError::bad_request("connection_id required"))?;
            let conn = runway_core::git_providers::connection(&state.db, &state.crypto, p, conn_id)
                .await?
                .ok_or_else(|| ApiError::bad_request("connection not found"))?;
            let client = runway_core::git_providers::Client::new(p, conn);
            let repo = client
                .repository(&body.repo_full_name)
                .await
                .map_err(|_| ApiError::bad_request("repository not accessible via connection"))?;
            repo_id = repo["id"].as_i64().unwrap_or(0);
            // Bitbucket's API base is api.bitbucket.org — clones go to
            // bitbucket.org itself.
            repo_base_url = if p == "bitbucket" {
                "https://bitbucket.org".into()
            } else {
                client.conn.base_url.clone()
            };
            match p {
                "gitea" => gitea_connection_id = Some(conn_id),
                "gitlab" => gitlab_connection_id = Some(conn_id),
                _ => bitbucket_connection_id = Some(conn_id),
            }
            if body.preset.is_none() && config.get("runner").is_none() {
                if let Ok(files) = client.list_root_files(&body.repo_full_name, &branch).await {
                    let pj = if files.iter().any(|f| f == "package.json") {
                        let (owner, name) =
                            body.repo_full_name.rsplit_once('/').unwrap_or(("", ""));
                        client
                            .file_text(owner, name, "package.json", &branch)
                            .await
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                    package_json = pj;
                    root_files = files;
                }
            }
        }
        _ => {
            return Err(ApiError::bad_request(format!(
                "provider '{provider}' not supported"
            )))
        }
    }

    // Auto-detect framework when neither preset nor explicit runner given.
    if body.preset.is_none() && config.get("runner").is_none() && !root_files.is_empty() {
        let refs: Vec<&str> = root_files.iter().map(String::as_str).collect();
        if let Some(preset) = runway_core::presets::detect(&refs, package_json.as_deref()) {
            apply_preset(&mut config, preset);
            runway_core::presets::adjust_for_pm(
                &mut config,
                runway_core::presets::package_manager(&refs),
            );
        } else if root_files.iter().any(|f| f == "Dockerfile") {
            // No framework matched but the repo ships a Dockerfile —
            // build it directly (docker build, image CMD serves).
            config
                .as_object_mut()
                .unwrap()
                .entry("dockerfile_path".to_string())
                .or_insert_with(|| "Dockerfile".into());
        }
    }

    let id = token_hex(16);
    let environments = json!([Environment::production(&branch)]);
    let project: Project = sqlx::query_as(
        "INSERT INTO project (
            id, team_id, name, description, repo_provider, repo_id, repo_full_name,
            repo_base_url, repo_branch, github_installation_id,
            gitea_connection_id, gitlab_connection_id, bitbucket_connection_id,
            config, environments, created_by_user_id
        ) VALUES ($1,$2,$3,'',$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
        RETURNING *",
    )
    .bind(&id)
    .bind(&team_id)
    .bind(&body.name)
    .bind(provider)
    .bind(repo_id)
    .bind(&body.repo_full_name)
    .bind(&repo_base_url)
    .bind(&branch)
    .bind(body.installation_id)
    .bind(gitea_connection_id)
    .bind(gitlab_connection_id)
    .bind(bitbucket_connection_id)
    .bind(&config)
    .bind(&environments)
    .bind(user.user.id)
    .fetch_one(&state.db)
    .await?;

    let mut project = project;
    let team_slug: String = sqlx::query_scalar("SELECT slug FROM team WHERE id = $1")
        .bind(&team_id)
        .fetch_one(&state.db)
        .await?;
    project.assign_slug(&state.db, &team_slug).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": project.id,
            "name": project.name,
            "slug": project.slug,
            "url": project.url(&state.settings),
        })),
    )
        .into_response())
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    Ok(Json(project_json(&state, &project)).into_response())
}

#[derive(Deserialize)]
pub struct PatchProject {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Shallow-merged into project.config.
    pub config: Option<Value>,
    /// Assign to a remote Docker node (null = local daemon).
    pub remote_node_id: Option<Option<String>>,
}

pub async fn patch(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchProject>,
) -> ApiResult<Response> {
    let mut project = accessible_project(&state, user.user.id, &id).await?;
    if let Some(name) = body.name {
        project.name = name;
    }
    if let Some(desc) = body.description {
        project.description = desc;
    }
    let mut edge_dirty = false;
    if let Some(patch) = body.config {
        if let (Some(obj), Some(patch)) = (project.config.as_object_mut(), patch.as_object()) {
            edge_dirty = patch.contains_key("firewall");
            for (k, v) in patch {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(node) = body.remote_node_id {
        // null clears the assignment; a value must reference a real node.
        if let Some(nid) = &node {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM remote_node WHERE id = $1)")
                    .bind(nid)
                    .fetch_one(&state.db)
                    .await?;
            if !exists {
                return Err(ApiError::bad_request("remote node not found"));
            }
        }
        project.remote_node_id = node;
    }
    let updated: Project = sqlx::query_as(
        "UPDATE project SET name = $2, description = $3, config = $4,
                remote_node_id = $5, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(&project.id)
    .bind(&project.name)
    .bind(&project.description)
    .bind(&project.config)
    .bind(&project.remote_node_id)
    .fetch_one(&state.db)
    .await?;
    // Firewall changes apply at the edge immediately — waiting for the
    // next deploy would leave a window open.
    if edge_dirty {
        rewrite_traefik(&state, &updated).await?;
    }
    Ok(Json(project_json(&state, &updated)).into_response())
}

fn project_json(state: &AppState, p: &Project) -> Value {
    json!({
        "id": p.id,
        "team_id": p.team_id,
        "name": p.name,
        "slug": p.slug,
        "description": p.description,
        "url": p.url(&state.settings),
        "repo_provider": p.repo_provider,
        "repo_id": p.repo_id,
        "repo_full_name": p.repo_full_name,
        "repo_branch": p.repo_branch,
        "repo_status": p.repo_status,
        "github_installation_id": p.github_installation_id,
        "gitea_connection_id": p.gitea_connection_id,
        "gitlab_connection_id": p.gitlab_connection_id,
        "bitbucket_connection_id": p.bitbucket_connection_id,
        "remote_node_id": p.remote_node_id,
        "config": p.config,
        "environments": p.environments,
        "status": p.status,
        "created_at": p.created_at,
        "updated_at": p.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Env vars
// ---------------------------------------------------------------------------

pub async fn get_env(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    // Keys returned; values masked except empty. Port keeps them hidden in
    // the API — full values visible only in the dashboard session flow.
    let vars = project.env_vars(&state.crypto)?;
    let masked: Vec<Value> = vars
        .iter()
        .map(|v| {
            json!({
                "key": v.key,
                "value": mask(&v.value),
                "environment": v.environment,
            })
        })
        .collect();
    Ok(Json(json!({ "env": masked })).into_response())
}

fn mask(value: &str) -> String {
    if value.len() <= 4 {
        "****".into()
    } else {
        format!("{}…{}", &value[..2], &value[value.len() - 2..])
    }
}

#[derive(Deserialize)]
pub struct EnvVarInput {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub environment: Option<String>,
}

/// Replace the full env var list (same semantics as devpush project env edit).
pub async fn put_env(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Vec<EnvVarInput>>,
) -> ApiResult<Response> {
    let mut project = accessible_project(&state, user.user.id, &id).await?;
    for v in &body {
        if v.key.is_empty() || !v.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ApiError::bad_request(format!(
                "invalid env key '{}'",
                v.key
            )));
        }
        if let Some(env) = &v.environment {
            if project
                .environments()
                .iter()
                .all(|e| &e.slug != env && &e.id != env)
            {
                return Err(ApiError::bad_request(format!(
                    "unknown environment '{env}'"
                )));
            }
        }
    }
    let vars: Vec<runway_core::models::EnvVar> = body
        .into_iter()
        .map(|v| runway_core::models::EnvVar {
            key: v.key,
            value: v.value,
            environment: v.environment,
        })
        .collect();
    project.set_env_vars(&state.crypto, &vars)?;
    sqlx::query("UPDATE project SET env_vars = $1, updated_at = now() WHERE id = $2")
        .bind(&project.env_vars)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct EnvPatchInput {
    pub key: String,
    /// Omit or `delete: true` to unset.
    pub value: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub delete: bool,
}

/// Upsert/delete individual env vars — values are masked on GET, so the CLI
/// cannot round-trip the full list; PATCH is the granular counterpart.
pub async fn patch_env(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Vec<EnvPatchInput>>,
) -> ApiResult<Response> {
    let mut project = accessible_project(&state, user.user.id, &id).await?;
    let mut vars = project.env_vars(&state.crypto)?;
    for p in &body {
        if p.key.is_empty() || !p.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ApiError::bad_request(format!(
                "invalid env key '{}'",
                p.key
            )));
        }
        if let Some(env) = &p.environment {
            if project
                .environments()
                .iter()
                .all(|e| &e.slug != env && &e.id != env)
            {
                return Err(ApiError::bad_request(format!(
                    "unknown environment '{env}'"
                )));
            }
        }
        vars.retain(|v| !(v.key == p.key && v.environment == p.environment));
        if !p.delete {
            let value = p
                .value
                .clone()
                .ok_or_else(|| ApiError::bad_request("value required unless delete=true"))?;
            vars.push(runway_core::models::EnvVar {
                key: p.key.clone(),
                value,
                environment: p.environment.clone(),
            });
        }
    }
    project.set_env_vars(&state.crypto, &vars)?;
    sqlx::query("UPDATE project SET env_vars = $1, updated_at = now() WHERE id = $2")
        .bind(&project.env_vars)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true, "count": vars.len() })).into_response())
}

// ---------------------------------------------------------------------------
// Deploy tokens
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateDeployToken {
    pub name: String,
    pub environment_id: Option<String>,
}

pub async fn create_deploy_token(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateDeployToken>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let (raw, hash) = DeployToken::generate();
    let token_id = token_hex(16);
    sqlx::query(
        "INSERT INTO deploy_token (id, project_id, name, token, environment_id, created_by_user_id)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&token_id)
    .bind(&project.id)
    .bind(&body.name)
    .bind(&hash)
    .bind(&body.environment_id)
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": token_id, "token": raw })),
    )
        .into_response())
}

pub async fn delete_deploy_token(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, token_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let _project = accessible_project(&state, user.user.id, &project_id).await?;
    sqlx::query("UPDATE deploy_token SET status = 'revoked' WHERE id = $1 AND project_id = $2")
        .bind(&token_id)
        .bind(&project_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Domains
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateDomain {
    pub hostname: String,
    /// route | 301 | 302 | 307 | 308
    #[serde(rename = "type", default = "default_route")]
    pub r#type: String,
    pub environment_id: Option<String>,
}

fn default_route() -> String {
    "route".into()
}

pub async fn list_domains(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let domains: Vec<Domain> =
        sqlx::query_as("SELECT * FROM domain WHERE project_id = $1 ORDER BY id")
            .bind(&project.id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(json!({ "domains": domains })).into_response())
}

pub async fn add_domain(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateDomain>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let hostname = body.hostname.trim().to_lowercase();
    if hostname.is_empty()
        || hostname.len() > 253
        || !hostname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(ApiError::bad_request("invalid hostname"));
    }
    if !["route", "301", "302", "307", "308"].contains(&body.r#type.as_str()) {
        return Err(ApiError::bad_request("invalid domain type"));
    }
    // DNS verification lands with the domain service (Phase 3); for now
    // domains start pending and are activated by verify.
    sqlx::query(
        "INSERT INTO domain (project_id, hostname, type, environment_id, status)
         VALUES ($1,$2,$3,$4,'pending')",
    )
    .bind(&project.id)
    .bind(&hostname)
    .bind(&body.r#type)
    .bind(&body.environment_id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::CREATED.into_response())
}

/// Mark a domain active (manual flow until DNS/CF verification lands).
pub async fn verify_domain(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, domain_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let res = sqlx::query(
        "UPDATE domain SET status = 'active', last_checked_at = now()
         WHERE id = $1 AND project_id = $2",
    )
    .bind(domain_id)
    .bind(&project.id)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("domain"));
    }
    runway_core::traefik::update_project_config(&state.db, &project, &state.settings, &[])
        .await
        .map_err(ApiError::from)?;
    Ok(Json(json!({ "ok": true })).into_response())
}

/// One-click Cloudflare assign: create the CNAME, add the hostname to
/// the instance tunnel ingress when one exists, mark active.
pub async fn assign_cloudflare_domain(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, domain_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let domain: Option<(String,)> =
        sqlx::query_as("SELECT hostname FROM domain WHERE id = $1 AND project_id = $2")
            .bind(domain_id)
            .bind(&project.id)
            .fetch_optional(&state.db)
            .await?;
    let Some((hostname,)) = domain else {
        return Err(ApiError::not_found("domain"));
    };

    // Team connection wins (devpush assign-dns); instance CF config is
    // the fallback. Tunnel presence decides the CNAME target either way.
    let conn: Option<runway_core::models::CloudflareConnection> =
        sqlx::query_as("SELECT * FROM cloudflare_connection WHERE team_id = $1")
            .bind(&project.team_id)
            .fetch_optional(&state.db)
            .await?;
    let (token, account_id, tunnel_id) = match &conn {
        Some(c) => (
            c.api_token_dec(&state.crypto).map_err(ApiError::from)?,
            c.account_id.clone(),
            c.tunnel_id.clone(),
        ),
        None => {
            let (Some(token), Some(account_id)) = (
                state.settings.cf_api_token.clone(),
                state.settings.cf_account_id.clone(),
            ) else {
                return Err(ApiError::bad_request(
                    "cloudflare not configured (team connection or CF_API_TOKEN/CF_ACCOUNT_ID)",
                ));
            };
            (
                token,
                account_id,
                runway_core::tunnel::read_tunnel_state(&state.settings.data_dir)
                    .await
                    .map(|t| t.tunnel_id),
            )
        }
    };

    let cf = runway_core::cloudflare::CloudflareClient::new(token);
    // Through the tunnel when it exists (CGNAT), else straight at the app.
    let target = tunnel_id
        .as_ref()
        .map(|t| format!("{t}.cfargotunnel.com"))
        .unwrap_or_else(|| state.settings.app_hostname.clone());

    let rec = cf
        .create_or_update_dns_record(&hostname, &target, true)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::bad_request(format!("no cloudflare zone covers {hostname}")))?;

    if let Some(t) = &tunnel_id {
        cf_tunnel_sync(&cf, &account_id, t, Some(&hostname), None).await;
    }

    sqlx::query(
        "UPDATE domain SET status = 'active', cloudflare_zone_id = $1,
         cloudflare_record_id = $2, last_checked_at = now()
         WHERE id = $3",
    )
    .bind(rec["zone_id"].as_str().unwrap_or(""))
    .bind(rec["record_id"].as_str().unwrap_or(""))
    .bind(domain_id)
    .execute(&state.db)
    .await?;
    runway_core::traefik::update_project_config(&state.db, &project, &state.settings, &[])
        .await
        .map_err(ApiError::from)?;
    Ok(Json(json!({ "ok": true, "target": target })).into_response())
}

/// Best-effort tunnel ingress update — never blocks the request.
async fn cf_tunnel_sync(
    cf: &runway_core::cloudflare::CloudflareClient,
    account_id: &str,
    tunnel_id: &str,
    add: Option<&str>,
    remove: Option<&str>,
) {
    if let Err(e) = runway_core::tunnel::sync_ingress(cf, account_id, tunnel_id, add, remove).await
    {
        tracing::warn!(error = %e, "tunnel ingress sync failed");
    }
}

pub async fn delete_domain(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, domain_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
    let domain: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT hostname, cloudflare_zone_id, cloudflare_record_id
         FROM domain WHERE id = $1 AND project_id = $2",
    )
    .bind(domain_id)
    .bind(&project.id)
    .fetch_optional(&state.db)
    .await?;

    // Best-effort upstream cleanup before the row goes. Team connection
    // first — the record lives in whichever CF account created it.
    if let Some((hostname, Some(zone_id), Some(record_id))) = &domain {
        let conn: Option<runway_core::models::CloudflareConnection> =
            sqlx::query_as("SELECT * FROM cloudflare_connection WHERE team_id = $1")
                .bind(&project.team_id)
                .fetch_optional(&state.db)
                .await?;
        let creds = match &conn {
            Some(c) => c
                .api_token_dec(&state.crypto)
                .ok()
                .map(|t| (t, c.account_id.clone(), c.tunnel_id.clone())),
            None => match (
                state.settings.cf_api_token.clone(),
                state.settings.cf_account_id.clone(),
            ) {
                (Some(t), Some(a)) => Some((
                    t, a, None, // instance tunnel id resolved below
                )),
                _ => None,
            },
        };
        if let Some((token, account_id, tunnel_id)) = creds {
            let cf = runway_core::cloudflare::CloudflareClient::new(token);
            let _ = cf.delete_dns_record(zone_id, record_id).await;
            let tunnel_id = tunnel_id.or_else(|| {
                // Instance tunnel — read blocking-free from disk state.
                std::fs::read_to_string(runway_core::tunnel::tunnel_state_path(
                    &state.settings.data_dir,
                ))
                .ok()
                .and_then(|raw| serde_json::from_str::<runway_core::tunnel::TunnelState>(&raw).ok())
                .map(|t| t.tunnel_id)
            });
            if let Some(t) = tunnel_id {
                cf_tunnel_sync(&cf, &account_id, &t, None, Some(hostname)).await;
            }
        }
    }

    sqlx::query("DELETE FROM domain WHERE id = $1 AND project_id = $2")
        .bind(domain_id)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    runway_core::traefik::update_project_config(&state.db, &project, &state.settings, &[])
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Project webhooks (outbound deployment events)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateWebhook {
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    /// deployment.* event names; empty = all.
    pub events: Option<Vec<String>>,
}

fn validate_webhook_input(name: &str, url: &str, events: &[String]) -> ApiResult<()> {
    if name.trim().is_empty() || name.len() > 100 {
        return Err(ApiError::bad_request("invalid webhook name"));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 2048 {
        return Err(ApiError::bad_request("webhook url must be http(s)"));
    }
    for e in events {
        if !runway_core::webhook::WEBHOOK_EVENTS.contains(&e.as_str()) {
            return Err(ApiError::bad_request(format!("unknown event '{e}'")));
        }
    }
    Ok(())
}

fn webhook_json(w: &runway_core::models::ProjectWebhook) -> Value {
    json!({
        "id": w.id,
        "project_id": w.project_id,
        "name": w.name,
        "url": w.url,
        "has_secret": w.secret.is_some(),
        "events": w.events,
        "status": w.status,
        "created_at": w.created_at,
    })
}

pub async fn list_webhooks(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let hooks: Vec<runway_core::models::ProjectWebhook> =
        sqlx::query_as("SELECT * FROM project_webhook WHERE project_id = $1 ORDER BY created_at")
            .bind(&project.id)
            .fetch_all(&state.db)
            .await?;
    Ok(
        Json(json!({ "webhooks": hooks.iter().map(webhook_json).collect::<Vec<_>>() }))
            .into_response(),
    )
}

pub async fn create_webhook(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateWebhook>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let events = body.events.unwrap_or_default();
    validate_webhook_input(&body.name, &body.url, &events)?;
    let secret_enc = match &body.secret {
        Some(s) if !s.is_empty() => Some(state.crypto.encrypt(s).map_err(ApiError::internal)?),
        _ => None,
    };
    let wid = runway_core::slugify::token_hex(16);
    sqlx::query(
        "INSERT INTO project_webhook (id, project_id, name, url, secret, events)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&wid)
    .bind(&project.id)
    .bind(body.name.trim())
    .bind(&body.url)
    .bind(&secret_enc)
    .bind(json!(events))
    .execute(&state.db)
    .await?;
    let hook: runway_core::models::ProjectWebhook =
        sqlx::query_as("SELECT * FROM project_webhook WHERE id = $1")
            .bind(&wid)
            .fetch_one(&state.db)
            .await?;
    Ok((StatusCode::CREATED, Json(webhook_json(&hook))).into_response())
}

pub async fn delete_webhook(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, webhook_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let res = sqlx::query("DELETE FROM project_webhook WHERE id = $1 AND project_id = $2")
        .bind(&webhook_id)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("webhook"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Cron jobs — scheduled deployments (devpush project cron handlers)
// ---------------------------------------------------------------------------

type CronRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    bool,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

fn cron_json(r: &CronRow) -> Value {
    json!({
        "id": r.0,
        "name": r.1,
        "schedule": r.2,
        "branch": r.3,
        "environment_id": r.4,
        "enabled": r.5,
        "last_run_at": r.6.map(|t| t.to_rfc3339()),
        "next_run_at": r.7.map(|t| t.to_rfc3339()),
    })
}

/// `GET /projects/{id}/cron` — the project's scheduled jobs.
pub async fn list_cron(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let rows: Vec<CronRow> = sqlx::query_as(
        "SELECT id, name, schedule, branch, environment_id, enabled,
                last_run_at, next_run_at
         FROM cron_job WHERE project_id = $1 ORDER BY created_at DESC",
    )
    .bind(&project.id)
    .fetch_all(&state.db)
    .await?;
    let jobs: Vec<Value> = rows.iter().map(cron_json).collect();
    Ok(Json(json!({ "cron_jobs": jobs })).into_response())
}

#[derive(Deserialize)]
pub struct CreateCronJob {
    pub name: String,
    /// "every N minutes" | "every N hours" | "*/N * * * *" | minutes.
    pub schedule: String,
    pub branch: Option<String>,
    pub environment_id: Option<String>,
}

/// `POST /projects/{id}/cron` — add a scheduled deployment.
pub async fn create_cron(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateCronJob>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let name = body.name.trim();
    let schedule = body.schedule.trim();
    if name.is_empty() || schedule.is_empty() {
        return Err(ApiError::bad_request("name and schedule are required"));
    }
    let interval = runway_core::cron::parse_schedule(schedule);
    if interval == 0 {
        return Err(ApiError::bad_request(
            "invalid schedule — use 'every N minutes', 'every N hours', '*/N * * * *', or minutes",
        ));
    }
    let branch = body.branch.unwrap_or_else(|| "main".into());
    let next_run = chrono::Utc::now() + chrono::Duration::minutes(interval as i64);
    let jid = token_hex(16);
    let row: CronRow = sqlx::query_as(
        "INSERT INTO cron_job (id, project_id, name, schedule, branch,
                               environment_id, next_run_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING id, name, schedule, branch, environment_id, enabled,
                   last_run_at, next_run_at",
    )
    .bind(&jid)
    .bind(&project.id)
    .bind(name)
    .bind(schedule)
    .bind(&branch)
    .bind(&body.environment_id)
    .bind(next_run)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(cron_json(&row))).into_response())
}

#[derive(Deserialize)]
pub struct PatchCronJob {
    pub enabled: Option<bool>,
}

/// `PATCH /projects/{id}/cron/{job_id}` — enable/disable.
pub async fn patch_cron(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, job_id)): Path<(String, String)>,
    Json(body): Json<PatchCronJob>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let Some(enabled) = body.enabled else {
        return Err(ApiError::bad_request("nothing to update"));
    };
    // Re-enabling a job whose next_run lapsed re-arms it: next tick
    // fires it and pushes next_run forward by the interval.
    let row: Option<CronRow> = sqlx::query_as(
        "UPDATE cron_job SET enabled = $3,
            next_run_at = CASE WHEN $3 AND (next_run_at IS NULL OR next_run_at < now())
                THEN now() ELSE next_run_at END,
            updated_at = now()
         WHERE id = $1 AND project_id = $2
         RETURNING id, name, schedule, branch, environment_id, enabled,
                   last_run_at, next_run_at",
    )
    .bind(&job_id)
    .bind(&project.id)
    .bind(enabled)
    .fetch_optional(&state.db)
    .await?;
    row.map(|r| Json(cron_json(&r)).into_response())
        .ok_or_else(|| ApiError::not_found("cron job"))
}

/// `DELETE /projects/{id}/cron/{job_id}` — remove.
pub async fn delete_cron(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, job_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let res = sqlx::query("DELETE FROM cron_job WHERE id = $1 AND project_id = $2")
        .bind(&job_id)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("cron job"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Redirect rules — path-level redirects via Traefik middlewares
// ---------------------------------------------------------------------------

type RedirectRow = (String, String, String, i32, bool, String);

fn redirect_json(r: &RedirectRow) -> Value {
    json!({
        "id": r.0,
        "source_path": r.1,
        "target_url": r.2,
        "status_code": r.3,
        "enabled": r.4,
        "created_at": r.5,
    })
}

/// Rebuild the project's Traefik config after rule mutations.
async fn rewrite_traefik(state: &AppState, project: &Project) -> ApiResult<()> {
    runway_core::traefik::update_project_config(&state.db, project, &state.settings, &[])
        .await
        .map_err(ApiError::from)
}

/// `GET /projects/{id}/redirects`.
pub async fn list_redirects(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let rows: Vec<RedirectRow> = sqlx::query_as(
        "SELECT id, source_path, target_url, status_code, enabled,
                created_at::text
         FROM redirect_rule WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(&project.id)
    .fetch_all(&state.db)
    .await?;
    let rules: Vec<Value> = rows.iter().map(redirect_json).collect();
    Ok(Json(json!({ "redirect_rules": rules })).into_response())
}

#[derive(Deserialize)]
pub struct CreateRedirect {
    /// Literal source path, e.g. `/old-blog`.
    pub source_path: String,
    /// Absolute target URL, e.g. `https://example.com/new`.
    pub target_url: String,
    /// 301/302/307/308 — default 301.
    pub status_code: Option<i32>,
}

/// `POST /projects/{id}/redirects`.
pub async fn create_redirect(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateRedirect>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let source = body.source_path.trim();
    let target = body.target_url.trim();
    if !source.starts_with('/') {
        return Err(ApiError::bad_request("source_path must start with /"));
    }
    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return Err(ApiError::bad_request("target_url must be absolute"));
    }
    let status_code = body.status_code.unwrap_or(301);
    if !matches!(status_code, 301 | 302 | 307 | 308) {
        return Err(ApiError::bad_request("status_code must be 301/302/307/308"));
    }
    let rid = token_hex(16);
    let row: RedirectRow = sqlx::query_as(
        "INSERT INTO redirect_rule (id, project_id, source_path, target_url,
                                    status_code)
         VALUES ($1,$2,$3,$4,$5)
         RETURNING id, source_path, target_url, status_code, enabled,
                   created_at::text",
    )
    .bind(&rid)
    .bind(&project.id)
    .bind(source)
    .bind(target)
    .bind(status_code)
    .fetch_one(&state.db)
    .await?;
    rewrite_traefik(&state, &project).await?;
    Ok((StatusCode::CREATED, Json(redirect_json(&row))).into_response())
}

#[derive(Deserialize)]
pub struct PatchRedirect {
    pub enabled: Option<bool>,
    pub source_path: Option<String>,
    pub target_url: Option<String>,
    pub status_code: Option<i32>,
}

/// `PATCH /projects/{id}/redirects/{rid}`.
pub async fn patch_redirect(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, rid)): Path<(String, String)>,
    Json(body): Json<PatchRedirect>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    if let Some(sc) = body.status_code {
        if !matches!(sc, 301 | 302 | 307 | 308) {
            return Err(ApiError::bad_request("status_code must be 301/302/307/308"));
        }
    }
    let row: Option<RedirectRow> = sqlx::query_as(
        "UPDATE redirect_rule SET
            enabled = COALESCE($3, enabled),
            source_path = COALESCE($4, source_path),
            target_url = COALESCE($5, target_url),
            status_code = COALESCE($6, status_code)
         WHERE id = $1 AND project_id = $2
         RETURNING id, source_path, target_url, status_code, enabled,
                   created_at::text",
    )
    .bind(&rid)
    .bind(&project.id)
    .bind(body.enabled)
    .bind(&body.source_path)
    .bind(&body.target_url)
    .bind(body.status_code)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("redirect rule"));
    };
    rewrite_traefik(&state, &project).await?;
    Ok(Json(redirect_json(&row)).into_response())
}

/// `DELETE /projects/{id}/redirects/{rid}`.
pub async fn delete_redirect(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, rid)): Path<(String, String)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let res = sqlx::query("DELETE FROM redirect_rule WHERE id = $1 AND project_id = $2")
        .bind(&rid)
        .bind(&project.id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("redirect rule"));
    }
    rewrite_traefik(&state, &project).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Export / import — devpush `project_export` / `project_import`
// ---------------------------------------------------------------------------

/// `GET /projects/{id}/export` — JSON dump of config + env vars +
/// redirect rules. Env values are decrypted in the export (devpush
/// parity — the file is the secrets container; treat it as such).
pub async fn export_project(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &id).await?;
    let vars = project.env_vars(&state.crypto)?;
    let rules: Vec<RedirectRow> = sqlx::query_as(
        "SELECT id, source_path, target_url, status_code, enabled,
                created_at::text
         FROM redirect_rule WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(&project.id)
    .fetch_all(&state.db)
    .await?;
    let body = json!({
        "version": "1.0",
        "project": {
            "name": project.name,
            "repo_provider": project.repo_provider,
            "repo_full_name": project.repo_full_name,
            "repo_base_url": project.repo_base_url,
            "config": project.config,
        },
        "environment_variables": vars,
        "redirect_rules": rules.iter().map(|r| json!({
            "source_path": r.1,
            "target_url": r.2,
            "status_code": r.3,
            "enabled": r.4,
        })).collect::<Vec<_>>(),
    });
    Ok((
        [(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}-export.json\"", project.name),
        )],
        Json(body),
    )
        .into_response())
}

/// `POST /projects/{id}/import` — merge an export into the project.
/// devpush parity: config keys are merged (imported wins), env vars are
/// appended when (key, environment) isn't already present, redirect
/// rules are added unconditionally.
pub async fn import_project(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Response> {
    let mut project = accessible_project(&state, user.user.id, &id).await?;

    // config merge
    if let Some(imported) = body
        .get("project")
        .and_then(|p| p.get("config"))
        .and_then(|c| c.as_object())
    {
        let cfg = project
            .config
            .as_object_mut()
            .ok_or_else(|| ApiError::bad_request("project config is not an object"))?;
        for (k, v) in imported {
            cfg.insert(k.clone(), v.clone());
        }
    }

    // env var merge — skip duplicates on (key, environment)
    if let Some(imported) = body.get("environment_variables").and_then(|v| v.as_array()) {
        let mut vars = project.env_vars(&state.crypto)?;
        let existing: std::collections::HashSet<(String, Option<String>)> = vars
            .iter()
            .map(|v| (v.key.clone(), v.environment.clone()))
            .collect();
        for ev in imported {
            let var: runway_core::models::EnvVar = serde_json::from_value(ev.clone())
                .map_err(|_| ApiError::bad_request("malformed environment_variables entry"))?;
            if var.key.is_empty() {
                return Err(ApiError::bad_request("env var key must not be empty"));
            }
            if !existing.contains(&(var.key.clone(), var.environment.clone())) {
                vars.push(var);
            }
        }
        project.set_env_vars(&state.crypto, &vars)?;
        sqlx::query("UPDATE project SET env_vars = $1, updated_at = now() WHERE id = $2")
            .bind(&project.env_vars)
            .bind(&project.id)
            .execute(&state.db)
            .await?;
    }

    // redirect rules — added unconditionally (devpush parity: no dedupe)
    let mut added = 0i64;
    if let Some(rules) = body.get("redirect_rules").and_then(|v| v.as_array()) {
        for r in rules {
            let source = r.get("source_path").and_then(|v| v.as_str()).unwrap_or("");
            let target = r.get("target_url").and_then(|v| v.as_str()).unwrap_or("");
            if source.is_empty() || target.is_empty() {
                return Err(ApiError::bad_request(
                    "redirect rule requires source_path and target_url",
                ));
            }
            let status_code = r.get("status_code").and_then(|v| v.as_i64()).unwrap_or(301) as i32;
            let enabled = r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            sqlx::query(
                "INSERT INTO redirect_rule (id, project_id, source_path,
                                           target_url, status_code, enabled)
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(token_hex(16))
            .bind(&project.id)
            .bind(source)
            .bind(target)
            .bind(status_code)
            .bind(enabled)
            .execute(&state.db)
            .await?;
            added += 1;
        }
        rewrite_traefik(&state, &project).await?;
    }

    sqlx::query("UPDATE project SET config = $1, updated_at = now() WHERE id = $2")
        .bind(&project.config)
        .bind(&project.id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({ "ok": true, "redirect_rules_added": added })).into_response())
}
