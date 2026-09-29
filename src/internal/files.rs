//! Writing the files under `.varda/` without losing them to a crash.

use anyhow::{Context, Result};
use std::path::Path;

/// Writes to a `.tmp` sibling, then renames it over `path`, so a crash
/// mid-write leaves the old file intact.
///
/// # Errors
///
/// Returns an error if the temporary file cannot be written or the rename
/// fails.
pub fn atomic_write<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)
        .with_context(|| format!("Failed to write temp file: {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}
