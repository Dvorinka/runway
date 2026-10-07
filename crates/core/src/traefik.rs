//! Traefik dynamic file provider writer.
//!
//! The app emits `<data_dir>/traefik/project_<id>.yml`; Traefik watches
//! the directory. Writes are atomic (temp file + rename) so Traefik never
//! reads a partial config.

use std::path::{Path, PathBuf};

use crate::error::Result;

pub fn config_path(data_dir: &str, project_id: &str) -> PathBuf {
    Path::new(data_dir)
        .join("traefik")
        .join(format!("project_{project_id}.yml"))
}

/// Write a YAML config atomically. (Phase 1: routers/services/middlewares
/// from aliases + domains, same shape as the reference implementation.)
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("yml.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
