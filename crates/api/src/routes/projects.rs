//! Project routes: create (from a GitHub repo), list, inspect, env vars,
//! deploy tokens, domains.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::models::{DeployToken, Domain, Environment, Project};
use runway_core::slugify::token_hex;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn user_projects(state: &AppState, user_id: i64) -> ApiResult<Vec<Project>> {
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
                "repo_status": p.repo_status,
                "environments": p.environments,
                "created_at": p.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "projects": items })).into_response())
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub name: String,
    /// GitHub repo numeric id.
    pub repo_id: i64,
    /// e.g. "acme/site".
    pub repo_full_name: String,
    pub installation_id: i64,
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
    let Some(gh) = &state.github else {
        return Err(ApiError::bad_request("GitHub App is not configured"));
    };
    let team_id = default_team_id(&state, &user.user).await?;
    let branch = body.branch.unwrap_or_else(|| "main".into());

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

    // Verify the repo is reachable via the installation.
    let token = gh
        .installation_token(&state.db, &state.crypto, body.installation_id)
        .await
        .map_err(|e| ApiError::bad_request(format!("installation token failed: {e}")))?;
    let _repo = gh
        .repository(&token, body.repo_id)
        .await
        .map_err(|_| ApiError::bad_request("repository not accessible via installation"))?;

    // Auto-detect framework when neither preset nor explicit runner given.
    if body.preset.is_none() && config.get("runner").is_none() {
        if let Ok(files) = gh.repo_files(&token, &body.repo_full_name, &branch).await {
            let refs: Vec<&str> = files.iter().map(String::as_str).collect();
            let pj = if refs.contains(&"package.json") {
                gh.file_text(&token, &body.repo_full_name, "package.json")
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            if let Some(preset) = runway_core::presets::detect(&refs, pj.as_deref()) {
                apply_preset(&mut config, preset);
                runway_core::presets::adjust_for_pm(
                    &mut config,
                    runway_core::presets::package_manager(&refs),
                );
            }
        }
    }

    let id = token_hex(16);
    let environments = json!([Environment::production(&branch)]);
    let project: Project = sqlx::query_as(
        "INSERT INTO project (
            id, team_id, name, description, repo_provider, repo_id, repo_full_name,
            repo_base_url, repo_branch, github_installation_id, config, environments,
            created_by_user_id
        ) VALUES ($1,$2,$3,'',$4,$5,$6,'https://github.com',$7,$8,$9,$10,$11)
        RETURNING *",
    )
    .bind(&id)
    .bind(&team_id)
    .bind(&body.name)
    .bind("github")
    .bind(body.repo_id)
    .bind(&body.repo_full_name)
    .bind(&branch)
    .bind(body.installation_id)
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
    if let Some(patch) = body.config {
        if let (Some(obj), Some(patch)) = (project.config.as_object_mut(), patch.as_object()) {
            for (k, v) in patch {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let updated: Project = sqlx::query_as(
        "UPDATE project SET name = $2, description = $3, config = $4, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(&project.id)
    .bind(&project.name)
    .bind(&project.description)
    .bind(&project.config)
    .fetch_one(&state.db)
    .await?;
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

pub async fn delete_domain(
    user: AuthUser,
    State(state): State<AppState>,
    Path((project_id, domain_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let project = accessible_project(&state, user.user.id, &project_id).await?;
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
