//! Avatar storage — `data_dir/uploads/avatars/{kind}_{id}.{ext}`.
//! Port of devpush's avatar upload, minus the image pipeline: uploads are
//! stored as received (2MB cap, image mime only) rather than re-encoded to
//! webp — no image crate in the workspace, browsers handle all formats.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// image content-type → stored extension. No svg: Pillow never accepted
/// it either, and inline svg is an XSS vector.
pub fn ext_for(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next().unwrap_or("").trim() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("uploads").join("avatars")
}

/// Replace any existing avatar (regardless of extension) with `bytes`.
pub async fn save(
    data_dir: &Path,
    kind: &str,
    id: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<()> {
    let ext = ext_for(content_type)
        .ok_or_else(|| Error::BadRequest("avatar must be png, jpeg, webp or gif".into()))?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(Error::BadRequest("avatar must be under 2MB".into()));
    }
    let dir = dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    remove(data_dir, kind, id).await?;
    tokio::fs::write(dir.join(format!("{kind}_{id}.{ext}")), bytes).await?;
    Ok(())
}

/// Locate the stored avatar file, if any. Returns (path, content-type).
pub async fn find(data_dir: &Path, kind: &str, id: &str) -> Option<(PathBuf, &'static str)> {
    let dir = dir(data_dir);
    for (ext, mime) in [
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("webp", "image/webp"),
        ("gif", "image/gif"),
    ] {
        let path = dir.join(format!("{kind}_{id}.{ext}"));
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Some((path, mime));
        }
    }
    None
}

pub async fn remove(data_dir: &Path, kind: &str, id: &str) -> Result<()> {
    if let Some((path, _)) = find(data_dir, kind, id).await {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}
