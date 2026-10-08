use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Returns a unique sibling path for an unpublished storage file.
#[must_use]
pub fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    path.with_file_name(name)
}

// Windows has no supported directory FlushFileBuffers equivalent; replace uses
// MoveFileExW WRITE_THROUGH to complete namespace publication before returning.
#[cfg_attr(
    not(unix),
    expect(
        clippy::unnecessary_wraps,
        clippy::missing_const_for_fn,
        reason = "the Unix build does real I/O here; keep one signature for both platforms"
    )
)]
pub fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub async fn sync_parent_async(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || sync_parent(&path))
        .await
        .map_err(io::Error::other)?
}

/// Publishes a forced temporary, preserving the previous target as a backup.
/// The caller must serialize writers and handle failure before releasing storage ownership.
// Util.safeReplaceOrMoveFile uses ten attempts per stage and checks the result.
pub fn safe_replace_file(target: &Path, new: &Path, backup: &Path) -> io::Result<()> {
    safe_replace_with(target, new, backup, replace, sync_parent)
}

fn retry(mut operation: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    const MAX_ATTEMPTS: usize = 10; // Util.safeReplaceOrMoveFile
    let mut result = Ok(());
    for _ in 0..MAX_ATTEMPTS {
        result = operation();
        if result.is_ok() {
            break;
        }
    }
    result
}

fn safe_replace_with(
    target: &Path,
    new: &Path,
    backup: &Path,
    mut move_file: impl FnMut(&Path, &Path) -> io::Result<()>,
    mut force_parent: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let had_target = target.try_exists()?;
    if had_target {
        retry(|| match fs::remove_file(backup) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        })?;
        retry(|| move_file(target, backup))?;
        if !backup.is_file() {
            return Err(io::Error::other("Player backup was not created"));
        }
        force_parent(target)?;
    }
    if let Err(error) = retry(|| move_file(new, target)) {
        if had_target {
            if let Err(restore_error) = retry(|| move_file(backup, target)) {
                tracing::error!("Failed to restore {}: {restore_error}", target.display());
            }
            // A failed rollback retains the forced backup for PlayerDataStorage.load.
            force_parent(target)?;
        }
        return Err(error);
    }
    if !target.is_file() {
        return Err(io::Error::other("Player replacement was not created"));
    }
    force_parent(target)
}

/// Owns an unpublished temporary; creation and writes must remain inside its blocking job.
#[derive(Debug)]
pub struct TemporaryFile {
    path: PathBuf,
    blocking_owner: bool,
    published: bool,
}

impl TemporaryFile {
    fn new(target: &Path) -> Self {
        Self {
            path: temporary_path(target),
            blocking_owner: true,
            published: false,
        }
    }
}

impl TemporaryFile {
    /// Creates, writes and forces a temporary in one owned blocking operation.
    pub async fn write(target: &Path, parts: Vec<bytes::Bytes>) -> io::Result<Self> {
        let target = target.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut temporary = Self::new(&target);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary.path)?;
            for part in parts {
                file.write_all(&part)?;
            }
            file.sync_all()?;
            temporary.blocking_owner = false;
            Ok(temporary)
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn publish(mut self, target: &Path) -> io::Result<()> {
        let target = target.to_path_buf();
        // Move cleanup ownership with the rename, so cancelling the waiter cannot
        // remove the source before an already queued rename executes.
        tokio::task::spawn_blocking(move || {
            self.blocking_owner = true;
            replace(&self.path, &target)?;
            self.published = true;
            drop(self);
            sync_parent(&target)
        })
        .await
        .map_err(io::Error::other)?
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let path = self.path.clone();
        let cleanup = move || {
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != io::ErrorKind::NotFound
            {
                tracing::error!("Failed to remove temporary {}: {error}", path.display());
            }
        };
        if self.blocking_owner {
            cleanup();
        } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(cleanup);
        } else {
            cleanup();
        }
    }
}

#[cfg(not(windows))]
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
pub fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_REPLACE_EXISTING: u32 = 1;
    const MOVEFILE_WRITE_THROUGH: u32 = 8;
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both buffers are live, NUL-terminated UTF-16 paths.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Deletes UUID temporaries only in recognized storage directories under this world lock.
pub fn recover_temporaries(directory: &Path) -> io::Result<()> {
    for player_path in ["players/data", "playerdata"] {
        let path = directory.join(player_path);
        clean_owned_directory(directory, &path, true);
    }
    for dimension in [
        "",
        "DIM-1",
        "DIM1",
        "dimensions/minecraft/overworld",
        "dimensions/minecraft/the_nether",
        "dimensions/minecraft/the_end",
    ] {
        let dim = directory.join(dimension);
        for kind in ["region", "entities", "poi"] {
            clean_owned_directory(directory, &dim.join(kind), false);
        }
    }
    Ok(())
}

fn clean_owned_directory(root: &Path, path: &Path, players: bool) {
    let result = has_ownership_boundary(root, path).and_then(|boundary| {
        if boundary {
            Ok(())
        } else {
            clean_storage_directory(path, players)
        }
    });
    if let Err(error) = result {
        tracing::warn!(
            "Failed to clean storage temporaries in {}: {error}",
            path.display()
        );
    }
}

fn has_ownership_boundary(root: &Path, path: &Path) -> io::Result<bool> {
    let mut current = path;
    while current != root {
        match fs::symlink_metadata(current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
            _ => {}
        }
        if current.join("session.lock").try_exists()? {
            return Ok(true);
        }
        let Some(parent) = current.parent() else {
            return Ok(true);
        };
        current = parent;
    }
    Ok(false)
}

fn clean_storage_directory(directory: &Path, players: bool) -> io::Result<()> {
    // Never recurse into arbitrary backup/plugin trees or through symlinks.
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || directory.join("session.lock").try_exists()? {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str().and_then(|n| n.strip_suffix(".tmp")) else {
            continue;
        };
        let Some((target, id)) = name.rsplit_once('.') else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let valid = if players {
            target
                .strip_suffix(".dat")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        } else {
            let parts: Vec<_> = target.split('.').collect();
            matches!(parts.as_slice(), ["r", x, z, "mca" | "linear" | "pump"] | ["c", x, z, "mcc"]
                if x.parse::<i32>().is_ok() && z.parse::<i32>().is_ok())
        };
        if valid {
            fs::remove_file(entry.path())?;
            sync_parent(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
