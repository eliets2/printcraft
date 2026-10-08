//! Writing redacted output: a temporary file next to the destination, owner-only, written in
//! full and read back, then renamed over the destination. The destination is never deleted
//! first, so a failure at any step leaves it as it was.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically. The temporary file is created exclusively (an existing
/// file or link of that name is never followed) with owner-only permissions where the platform
/// has them, flushed, and compared with `bytes` before the rename.
///
/// - A new file stays owner-only: redacted output is sensitive until its owner decides otherwise.
///   Replacing an existing file keeps that file's permissions (its owner already decided), on
///   Unix. On Windows the file gets the default access rights of its folder: nothing here sets an
///   ACL, so "owner-only" is a Unix guarantee.
/// - A destination that is a symbolic link is refused: the rename would replace the link, not the
///   file it points to, and which of the two was meant is not ours to guess.
/// - On Unix the directory is flushed after the rename, so the new name survives a crash.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file path"))?;
    let existing = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the destination is a symbolic link; save to the file it points to"));
        }
        Ok(m) => Some(m),
        Err(_) => None,
    };
    let tmp = dir.join(format!(".{}.pdfcraft-{}-{}.tmp", name.to_string_lossy(), std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        if let Some(m) = existing.as_ref().filter(|m| m.is_file()) {
            f.set_permissions(m.permissions())?;
        }
        f.sync_all()?;
        drop(f);
        if std::fs::read(&tmp)? != bytes {
            return Err(std::io::Error::other("the file read back differs from what was written"));
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    // The file is in place; failing to flush the directory entry must not report a failed save.
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    result
}
