//! Writing the files under `.varda/` without losing them to a crash.

use anyhow::{Context, Result};
use std::path::Path;

/// Atomic file write: writes to a `.tmp` sibling then renames into place.
/// Prevents data loss if the process crashes mid-write.
///
/// # Errors
///
/// Returns an error if the temporary sibling file cannot be written (missing
/// parent directory, permissions, disk full) or if renaming it over `path`
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
