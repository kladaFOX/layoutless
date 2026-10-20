//! Files only the owner can read: the learned words are derived from what
//! the user types, so other local users must not see them.

use std::fs::{DirBuilder, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Create `dir` (and parents) owner-only, and tighten it if it already exists.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    std::fs::set_permissions(dir, Permissions::from_mode(0o700))
}

/// Tighten an existing file to owner-only; a missing file is fine.
pub fn restrict_file(path: &Path) -> std::io::Result<()> {
    match std::fs::set_permissions(path, Permissions::from_mode(0o600)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Write `contents` atomically (temp file + rename), owner-only from the
/// moment the temp file exists.
pub fn write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        ensure_dir(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp); // a stale one may have wider permissions
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}
