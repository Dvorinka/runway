//! Core domain types.
//!
//! The Postgres schema ports ~1:1 from the devpush reference
//! (`app/models.py`): user, team, project, deployment, alias, domain,
//! connections, storage, tokens. Structs land here as the schema does.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Prepare,
    Deploy,
    Finalize,
    Fail,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentConclusion {
    Succeeded,
    Failed,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoProvider {
    Github,
    Gitea,
    Gitlab,
    Bitbucket,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub environment_id: String,
    pub branch: String,
    pub commit_sha: String,
    pub status: DeploymentStatus,
    pub conclusion: Option<DeploymentConclusion>,
    pub container_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub concluded_at: Option<DateTime<Utc>>,
}
