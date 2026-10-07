//! File-tail log storage — replaces Loki + Alloy.
//!
//! Each deployment appends to `<data_dir>/logs/<deployment_id>.log`.
//! Lines are also broadcast on `deployment:{id}:logs` for SSE streaming.
//! Line format on disk: `<rfc3339>\t<stream>\t<text>`.

use std::path::{Path, PathBuf};

use chrono::Utc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::Result;
use crate::events::EventBus;

#[derive(Clone)]
pub struct LogStore {
    dir: PathBuf,
    bus: EventBus,
}

impl LogStore {
    pub fn new(data_dir: &str, bus: EventBus) -> Self {
        Self {
            dir: Path::new(data_dir).join("logs"),
            bus,
        }
    }

    pub fn path(&self, deployment_id: &str) -> PathBuf {
        self.dir.join(format!("{deployment_id}.log"))
    }

    /// Append a build/system log line; broadcasts to SSE subscribers.
    pub async fn append(&self, deployment_id: &str, stream: &str, text: &str) -> Result<()> {
        let line = format!("{}\t{}\t{}", Utc::now().to_rfc3339(), stream, text);
        tokio::fs::create_dir_all(&self.dir).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(deployment_id))
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        self.bus.publish_line(&scope(deployment_id), &line);
        Ok(())
    }

    /// Convenience wrapper for status/progress lines.
    pub async fn info(&self, deployment_id: &str, text: &str) {
        if let Err(e) = self.append(deployment_id, "system", text).await {
            tracing::warn!(deployment_id, error = %e, "failed to write deployment log");
        }
    }

    /// Append a raw container-log line (stdout/stderr tail).
    pub async fn append_raw(&self, deployment_id: &str, stream: &str, text: &str) {
        if let Err(e) = self.append(deployment_id, stream, text).await {
            tracing::warn!(deployment_id, error = %e, "failed to write container log");
        }
    }

    /// Read the last `n` lines of a deployment log.
    pub async fn tail(&self, deployment_id: &str, n: usize) -> Result<Vec<String>> {
        let path = self.path(deployment_id);
        if !path.exists() {
            return Ok(vec![]);
        }
        let file = tokio::fs::File::open(path).await?;
        let mut reader = BufReader::new(file).lines();
        let mut lines = std::collections::VecDeque::with_capacity(n.min(8192));
        while let Some(line) = reader.next_line().await? {
            if lines.len() == n {
                lines.pop_front();
            }
            lines.push_back(line);
        }
        Ok(lines.into_iter().collect())
    }
}

pub fn scope(deployment_id: &str) -> String {
    format!("deployment:{deployment_id}:logs")
}
