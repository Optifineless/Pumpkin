use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

static HELD_LOCKS: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Holds exclusive ownership of a world's session.lock until dropped.
pub struct SessionLock {
    file: Option<File>,
    path: PathBuf,
}

/// Acquires vanilla's session.lock; keep the returned value alive through all world I/O.
pub fn acquire(world_dir: impl AsRef<Path>) -> io::Result<SessionLock> {
    let world_dir = world_dir.as_ref();
    std::fs::create_dir_all(world_dir)?;
    let path = world_dir.canonicalize()?.join("session.lock");
    let mut held = HELD_LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if held.contains(&path) {
        return Err(already_locked(&path));
    }
    // DirectoryLock.create writes the UTF-8 snowman and forces the lock file.
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    try_lock(&file).map_err(|error| {
        if matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::PermissionDenied
        ) {
            already_locked(&path)
        } else {
            error
        }
    })?;
    file.write_all("☃".as_bytes())?;
    file.sync_all()?;
    held.insert(path.clone());
    Ok(SessionLock {
        file: Some(file),
        path,
    })
}

fn already_locked(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        format!(
            "{}: already locked by another Minecraft instance",
            path.display()
        ),
    )
}

#[cfg(unix)]
fn try_lock(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // Java FileChannel uses POSIX record locks, not flock. Linux OFD locks conflict
    // with Java's locks and remain held when unrelated descriptors are closed.
    // SAFETY: flock is a C integer-only struct; zero initializes its unused fields.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    #[cfg(target_os = "linux")]
    let command = libc::F_OFD_SETLK;
    #[cfg(not(target_os = "linux"))]
    let command = libc::F_SETLK;
    // SAFETY: the descriptor is live and lock points to a valid flock for this call.
    if unsafe { libc::fcntl(file.as_raw_fd(), command, &raw const lock) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn try_lock(file: &File) -> io::Result<()> {
    file.try_lock().map_err(io::Error::from)
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        drop(self.file.take());
        HELD_LOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_holder_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let lock = acquire(directory.path()).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("session.lock")).unwrap(),
            "☃".as_bytes()
        );
        assert!(acquire(directory.path()).is_err());
        drop(lock);
        let _lock = acquire(directory.path()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn conflicts_with_vanilla_posix_record_lock() {
        use std::os::fd::AsRawFd;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.lock");
        let file = File::create(&path).unwrap();
        // SAFETY: flock contains only C integer fields.
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_WRLCK as _;
        lock.l_whence = libc::SEEK_SET as _;
        // SAFETY: file is live and lock points to a valid flock.
        let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &raw const lock) };
        assert_eq!(result, 0);
        assert!(acquire(directory.path()).is_err());
        drop(file);
        let _lock = acquire(directory.path()).unwrap();
    }
}
