//! Team routes: list/get for the authenticated user, plus team-level
//! webhook CRUD (port of devpush team webhooks — events fan out to every
//! project or a `project_ids` subset).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use runway_core::models::TeamWebhook;
use runway_core::webhook::WEBHOOK_EVENTS;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

async fn accessible_team(state: &AppState, user_id: i64, id: &str) -> ApiResult<String> {
    let team: Option<(String,)> = sqlx::query_as(
        "SELECT t.id FROM team t
         JOIN team_member tm ON tm.team_id = t.id
         WHERE tm.user_id = $1 AND t.id = $2 AND t.status != 'deleted'",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    team.map(|(id,)| id)
        .ok_or_else(|| ApiError::not_found("team"))
}

pub async fn list(user: AuthUser, State(state): State<AppState>) -> ApiResult<Response> {
    let teams: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT t.id, t.name, t.slug, tm.role FROM team t
         JOIN team_member tm ON tm.team_id = t.id
         WHERE tm.user_id = $1 AND t.status != 'deleted' ORDER BY t.name",
    )
    .bind(user.user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "teams": teams.iter().map(|(id, name, slug, role)| json!({
            "id": id, "name": name, "slug": slug, "role": role,
        })).collect::<Vec<_>>()
    }))
    .into_response())
}

pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let team: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, slug FROM team WHERE id = $1")
            .bind(&team_id)
            .fetch_optional(&state.db)
            .await?;
    let members: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, tm.role FROM team_member tm
         JOIN \"user\" u ON u.id = tm.user_id WHERE tm.team_id = $1",
    )
    .bind(&team_id)
    .fetch_all(&state.db)
    .await?;
    let Some((id, name, slug)) = team else {
        return Err(ApiError::not_found("team"));
    };
    Ok(Json(json!({
        "team": {
            "id": id, "name": name, "slug": slug,
            "members": members.iter().map(|(uid, uname, role)| json!({
                "user_id": uid, "username": uname, "role": role,
            })).collect::<Vec<_>>(),
        }
    }))
    .into_response())
}

// -- Team webhooks -----------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateTeamWebhook {
    pub name: String,
    pub url: String,
    pub secret: Option<String>,
    /// deployment.* event names; empty = all.
    pub events: Option<Vec<String>>,
    /// Restrict delivery to these project ids; None = all team projects.
    pub project_ids: Option<Vec<String>>,
}

fn webhook_json(w: &TeamWebhook) -> Value {
    json!({
        "id": w.id,
        "team_id": w.team_id,
        "name": w.name,
        "url": w.url,
        "has_secret": w.secret.is_some(),
        "events": w.events,
        "project_ids": w.project_ids,
        "status": w.status,
        "created_at": w.created_at,
    })
}

pub async fn list_webhooks(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let hooks: Vec<TeamWebhook> =
        sqlx::query_as("SELECT * FROM team_webhook WHERE team_id = $1 ORDER BY created_at")
            .bind(&team_id)
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
    Json(body): Json<CreateTeamWebhook>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let events = body.events.unwrap_or_default();
    if body.name.trim().is_empty() || body.name.len() > 100 {
        return Err(ApiError::bad_request("invalid webhook name"));
    }
    if !(body.url.starts_with("http://") || body.url.starts_with("https://"))
        || body.url.len() > 2048
    {
        return Err(ApiError::bad_request("webhook url must be http(s)"));
    }
    for e in &events {
        if !WEBHOOK_EVENTS.contains(&e.as_str()) {
            return Err(ApiError::bad_request(format!("unknown event '{e}'")));
        }
    }
    let secret_enc = match &body.secret {
        Some(s) if !s.is_empty() => Some(state.crypto.encrypt(s).map_err(ApiError::internal)?),
        _ => None,
    };
    let wid = runway_core::slugify::token_hex(16);
    sqlx::query(
        "INSERT INTO team_webhook (id, team_id, name, url, secret, events, project_ids, created_by_user_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(&wid)
    .bind(&team_id)
    .bind(body.name.trim())
    .bind(&body.url)
    .bind(&secret_enc)
    .bind(json!(events))
    .bind(body.project_ids.as_ref().map(|ids| json!(ids)))
    .bind(user.user.id)
    .execute(&state.db)
    .await?;
    let hook: TeamWebhook = sqlx::query_as("SELECT * FROM team_webhook WHERE id = $1")
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
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let res = sqlx::query("DELETE FROM team_webhook WHERE id = $1 AND team_id = $2")
        .bind(&webhook_id)
        .bind(&team_id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("webhook"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// -- Team CRUD, members, invites ----------------------------------------------
// Phase 5. devpush team.py ported: slug from name, owner membership on
// create, role-gated mutations (owner/admin), invite by email with
// 30-day expiry, accept creates membership + marks accepted.

#[derive(Deserialize)]
pub struct CreateTeam {
    pub name: String,
}

pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTeam>,
) -> ApiResult<Response> {
    let name = body.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad_request("name must be 1-100 chars"));
    }
    let id = runway_core::slugify::token_hex(16);
    let mut slug = runway_core::slugify::slugify(name, 40);
    if slug.is_empty() {
        slug = "team".into();
    }
    // Unique slug — append suffixes until free (devpush does the same).
    let mut candidate = slug.clone();
    for i in 2.. {
        let exists: Option<(String,)> = sqlx::query_as("SELECT slug FROM team WHERE slug = $1")
            .bind(&candidate)
            .fetch_optional(&state.db)
            .await?;
        if exists.is_none() {
            break;
        }
        candidate = format!("{slug}-{i}");
    }
    sqlx::query("INSERT INTO team (id, name, slug, created_by_user_id) VALUES ($1,$2,$3,$4)")
        .bind(&id)
        .bind(name)
        .bind(&candidate)
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO team_member (team_id, user_id, role) VALUES ($1,$2,'owner')")
        .bind(&id)
        .bind(user.user.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "team": { "id": id, "name": name, "slug": candidate } })).into_response())
}

#[derive(Deserialize)]
pub struct UpdateTeam {
    pub name: Option<String>,
}

pub async fn update(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTeam>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    if let Some(name) = &body.name {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            return Err(ApiError::bad_request("name must be 1-100 chars"));
        }
        sqlx::query("UPDATE team SET name = $1, updated_at = now() WHERE id = $2")
            .bind(name)
            .bind(&team_id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(json!({ "ok": true })).into_response())
}

/// Soft-delete (devpush `status='deleted'`). Owner only.
pub async fn delete(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    if role != "owner" {
        return Err(ApiError::forbidden("team owner role required"));
    }
    sqlx::query("UPDATE team SET status = 'deleted', updated_at = now() WHERE id = $1")
        .bind(&team_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// -- Members -----------------------------------------------------------------

#[derive(Deserialize)]
pub struct UpdateMember {
    pub role: String,
}

pub async fn update_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, member_id)): Path<(String, i64)>,
    Json(body): Json<UpdateMember>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    if !matches!(body.role.as_str(), "owner" | "admin" | "member") {
        return Err(ApiError::bad_request("role must be owner|admin|member"));
    }
    // Never demote the last owner.
    if body.role != "owner" {
        let (owners,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM team_member WHERE team_id = $1 AND role = 'owner'",
        )
        .bind(&team_id)
        .fetch_one(&state.db)
        .await?;
        let (is_owner,): (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM team_member WHERE team_id=$1 AND user_id=$2 AND role='owner')",
        )
        .bind(&team_id)
        .bind(member_id)
        .fetch_one(&state.db)
        .await?;
        if is_owner && owners <= 1 {
            return Err(ApiError::bad_request("cannot demote the last owner"));
        }
    }
    let res = sqlx::query("UPDATE team_member SET role = $1 WHERE team_id = $2 AND user_id = $3")
        .bind(&body.role)
        .bind(&team_id)
        .bind(member_id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("member"));
    }
    Ok(Json(json!({ "ok": true })).into_response())
}

/// Remove a member — admin removes anyone, any member can leave.
/// Owner cannot leave while others remain (devpush parity).
pub async fn remove_member(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, member_id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    let self_remove = member_id == user.user.id;
    if !self_remove {
        crate::routes::cloudflare::require_admin(&role)?;
    }
    if self_remove && role == "owner" {
        let (owners,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM team_member WHERE team_id = $1 AND role = 'owner'",
        )
        .bind(&team_id)
        .fetch_one(&state.db)
        .await?;
        let (others,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM team_member WHERE team_id = $1 AND user_id != $2")
                .bind(&team_id)
                .bind(member_id)
                .fetch_one(&state.db)
                .await?;
        if owners <= 1 && others > 0 {
            return Err(ApiError::bad_request(
                "last owner cannot leave while members remain",
            ));
        }
    }
    sqlx::query("DELETE FROM team_member WHERE team_id = $1 AND user_id = $2")
        .bind(&team_id)
        .bind(member_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// -- Invites -----------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateInvite {
    pub email: String,
    pub role: Option<String>,
}

pub async fn list_invites(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let team_id = accessible_team(&state, user.user.id, &id).await?;
    let rows: Vec<(
        String,
        String,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, email, role, status, expires_at FROM team_invite
         WHERE team_id = $1 ORDER BY created_at DESC",
    )
    .bind(&team_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "invites": rows.iter().map(|(id, email, role, status, exp)| json!({
            "id": id, "email": email, "role": role, "status": status,
            "expires_at": exp,
        })).collect::<Vec<_>>()
    }))
    .into_response())
}

pub async fn create_invite(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateInvite>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 320 {
        return Err(ApiError::bad_request("invalid email"));
    }
    let role = body.role.unwrap_or_else(|| "member".into());
    if !matches!(role.as_str(), "owner" | "admin" | "member") {
        return Err(ApiError::bad_request("role must be owner|admin|member"));
    }
    // Already a member?
    let already: Option<(i64,)> = sqlx::query_as(
        "SELECT tm.user_id FROM team_member tm JOIN \"user\" u ON u.id = tm.user_id
         WHERE tm.team_id = $1 AND lower(u.email) = $2",
    )
    .bind(&team_id)
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;
    if already.is_some() {
        return Err(ApiError::bad_request("user is already a member"));
    }
    let iid = runway_core::slugify::token_hex(16);
    sqlx::query(
        "INSERT INTO team_invite (id, team_id, email, role, inviter_id)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(&iid)
    .bind(&team_id)
    .bind(&email)
    .bind(&role)
    .bind(user.user.id)
    .execute(&state.db)
    .await?;

    // Best-effort email — dev-mode logs it when SMTP isn't configured.
    let (team_name,): (String,) = sqlx::query_as("SELECT name FROM team WHERE id = $1")
        .bind(&team_id)
        .fetch_one(&state.db)
        .await?;
    let link = format!("https://{}/invites/{iid}", state.settings.app_hostname);
    let _ = runway_core::mail::send(
        &state.settings,
        &email,
        &format!("You're invited to {team_name} on Runway"),
        &format!(
            "{} invited you to join {team_name} (role: {role}).\n\nAccept: {link}\n",
            user.user.username
        ),
    )
    .await;

    Ok(Json(json!({ "invite": { "id": iid, "email": email, "role": role } })).into_response())
}

/// Revoke (admin) — marks status='revoked' rather than deleting, like devpush.
pub async fn revoke_invite(
    user: AuthUser,
    State(state): State<AppState>,
    Path((id, invite_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (team_id, role) = crate::routes::cloudflare::team_role(&state, user.user.id, &id).await?;
    crate::routes::cloudflare::require_admin(&role)?;
    let res = sqlx::query(
        "UPDATE team_invite SET status = 'revoked'
         WHERE id = $1 AND team_id = $2 AND status = 'pending'",
    )
    .bind(&invite_id)
    .bind(&team_id)
    .execute(&state.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("invite"));
    }
    Ok(Json(json!({ "ok": true })).into_response())
}

/// Accept — caller's email must match the invite. Creates membership.
pub async fn accept_invite(
    user: AuthUser,
    State(state): State<AppState>,
    Path(invite_id): Path<String>,
) -> ApiResult<Response> {
    let inv: Option<(String, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT team_id, email, role, expires_at FROM team_invite
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(&invite_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((team_id, email, role, expires)) = inv else {
        return Err(ApiError::not_found("invite"));
    };
    if expires < chrono::Utc::now() {
        return Err(ApiError::bad_request("invite expired"));
    }
    let user_email = user.user.email.to_lowercase();
    if user_email != email {
        return Err(ApiError::forbidden(
            "invite is addressed to a different email",
        ));
    }
    sqlx::query(
        "INSERT INTO team_member (team_id, user_id, role) VALUES ($1,$2,$3)
         ON CONFLICT (team_id, user_id) DO NOTHING",
    )
    .bind(&team_id)
    .bind(user.user.id)
    .bind(&role)
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE team_invite SET status = 'accepted' WHERE id = $1")
        .bind(&invite_id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true, "team_id": team_id })).into_response())
}
