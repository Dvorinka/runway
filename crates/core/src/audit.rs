//! Audit + notification writes — port of devpush `AuditLog.log` /
//! `Notification.create`. Best-effort by design: callers drop the result;
//! audit must never fail the request it observes.

use sqlx::PgPool;

pub struct Audit {
    pub action: &'static str,
    pub user_id: Option<i64>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub resource_type: Option<&'static str>,
    pub resource_id: Option<String>,
    pub detail: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

impl Audit {
    pub fn new(action: &'static str) -> Self {
        Self {
            action,
            user_id: None,
            team_id: None,
            project_id: None,
            resource_type: None,
            resource_id: None,
            detail: None,
            ip_address: None,
            user_agent: None,
        }
    }
}

/// Insert an audit row; failures are logged, never propagated.
pub async fn log(db: &PgPool, a: Audit) {
    if let Err(e) = sqlx::query(
        "INSERT INTO audit_log
         (user_id, team_id, project_id, action, resource_type, resource_id,
          detail, ip_address, user_agent)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(a.user_id)
    .bind(&a.team_id)
    .bind(&a.project_id)
    .bind(a.action)
    .bind(a.resource_type)
    .bind(&a.resource_id)
    .bind(&a.detail)
    .bind(&a.ip_address)
    .bind(
        a.user_agent
            .map(|u| u.chars().take(256).collect::<String>()),
    )
    .execute(db)
    .await
    {
        tracing::warn!(error = %e, "audit write failed");
    }
}

/// One notification's payload — kept small; `user_id` is per-recipient.
#[derive(Default)]
pub struct Notify<'a> {
    pub body: Option<&'a str>,
    pub link: Option<&'a str>,
    pub team_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
}

/// Insert a notification for one user; best-effort.
pub async fn notify(db: &PgPool, user_id: i64, ty: &str, title: &str, n: &Notify<'_>) {
    if let Err(e) = sqlx::query(
        "INSERT INTO notification (user_id, team_id, project_id, type, title, body, link)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(user_id)
    .bind(n.team_id)
    .bind(n.project_id)
    .bind(ty)
    .bind(title)
    .bind(n.body)
    .bind(n.link)
    .execute(db)
    .await
    {
        tracing::warn!(error = %e, "notification write failed");
    }
}

/// Notify every member of a team (deploy events, invites, ...).
pub async fn notify_team(db: &PgPool, team_id: &str, ty: &str, title: &str, mut n: Notify<'_>) {
    n.team_id = Some(team_id);
    let members: Vec<(i64,)> =
        match sqlx::query_as("SELECT user_id FROM team_member WHERE team_id = $1")
            .bind(team_id)
            .fetch_all(db)
            .await
        {
            Ok(m) => m,
            Err(_) => return,
        };
    for (uid,) in members {
        notify(db, uid, ty, title, &n).await;
    }
}
