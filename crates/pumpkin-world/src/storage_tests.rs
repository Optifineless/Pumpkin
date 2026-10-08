use super::*;

#[test]
fn failed_player_rollback_keeps_backup_for_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("player.dat");
    let backup = directory.path().join("player.dat_old");
    let temporary = directory.path().join("player.tmp");
    fs::write(&target, b"previous inventory").unwrap();
    fs::write(&temporary, b"new inventory").unwrap();
    let mut attempts = 0;
    let result = safe_replace_with(
        &target,
        &temporary,
        &backup,
        |from, to| {
            if from == target {
                return replace(from, to);
            }
            attempts += 1;
            Err(io::Error::other(
                "Injected replacement and rollback failure",
            ))
        },
        sync_parent,
    );
    assert!(result.is_err());
    assert_eq!(attempts, 20); // Util.safeReplaceOrMoveFile: 10 publication + 10 rollback attempts.
    assert_eq!(fs::read(&backup).unwrap(), b"previous inventory");
    assert!(!target.exists());
    assert_eq!(fs::read(&temporary).unwrap(), b"new inventory");
}

#[test]
fn ordinary_failure_and_restart_remove_only_unpublished_temporaries() {
    let directory = tempfile::tempdir().unwrap();
    let region = directory.path().join("region");
    fs::create_dir(&region).unwrap();
    let target = region.join("r.0.0.pump");
    let temporary = TemporaryFile::new(&target);
    let path = temporary.path.clone();
    fs::write(&path, b"partial").unwrap();
    drop(temporary);
    assert!(!path.exists());
    let interrupted = temporary_path(&target);
    fs::write(&interrupted, b"partial").unwrap();
    let unrelated = directory.path().join("keep.tmp");
    fs::write(&unrelated, b"unrelated").unwrap();
    recover_temporaries(directory.path()).unwrap();
    assert!(!interrupted.exists());
    assert!(unrelated.exists());
}

#[test]
fn recovery_respects_names_directories_and_nested_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let owned = root.join("dimensions/minecraft/overworld/region");
    let nested = root.join("DIM-1/region");
    fs::create_dir_all(&owned).unwrap();
    fs::create_dir_all(&nested).unwrap();
    fs::write(root.join("DIM-1/session.lock"), b"owned elsewhere").unwrap();
    let expected = temporary_path(&owned.join("r.-1.2.mca"));
    let unrelated = temporary_path(&owned.join("notes.mca"));
    let foreign = temporary_path(&nested.join("r.0.0.mca"));
    let backup = root.join("backups/region");
    fs::create_dir_all(&backup).unwrap();
    let backup = temporary_path(&backup.join("r.0.0.mca"));
    for path in [&expected, &unrelated, &foreign, &backup] {
        fs::write(path, b"pending").unwrap();
    }
    recover_temporaries(root).unwrap();
    assert!(!expected.exists());
    for path in [unrelated, foreign, backup] {
        assert!(path.exists(), "removed {}", path.display());
    }
}

#[test]
fn cancellation_before_queued_open_keeps_cleanup_with_the_operation() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("r.0.0.pump");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (release, held) = std::sync::mpsc::channel();
    let (blocked, started) = std::sync::mpsc::channel();
    runtime.spawn_blocking(move || {
        blocked.send(()).unwrap();
        held.recv().unwrap();
    });
    started.recv().unwrap();
    let (ready, ready_rx) = tokio::sync::oneshot::channel();
    let task = runtime.spawn(async move {
        ready.send(()).unwrap();
        TemporaryFile::write(&target, vec![bytes::Bytes::from_static(b"unpublished")]).await
    });
    runtime.block_on(ready_rx).unwrap();
    task.abort();
    assert!(runtime.block_on(task).unwrap_err().is_cancelled());
    release.send(()).unwrap();
    // Let the queued open and its cleanup finish while the runtime remains alive.
    // Forced runtime teardown, like process death, is handled by startup recovery.
    runtime.block_on(async {
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        tokio::task::spawn_blocking(|| ()).await.unwrap();
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn failed_force_after_backup_or_replacement_keeps_recoverable_player_data() {
    for failing_force in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("player.dat");
        let backup = directory.path().join("player.dat_old");
        let temporary = directory.path().join("player.tmp");
        fs::write(&target, b"previous inventory").unwrap();
        fs::write(&temporary, b"new inventory").unwrap();
        let mut forces = 0;
        let result = safe_replace_with(&target, &temporary, &backup, replace, |path| {
            forces += 1;
            if forces == failing_force {
                Err(io::Error::other("Injected force failure"))
            } else {
                sync_parent(path)
            }
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&backup).unwrap(), b"previous inventory");
        if failing_force == 2 {
            assert_eq!(fs::read(&target).unwrap(), b"new inventory");
        }
    }
}

#[cfg(unix)]
#[test]
fn unreadable_storage_directory_does_not_abort_other_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let region = directory.path().join("region");
    let entities = directory.path().join("entities");
    fs::create_dir(&region).unwrap();
    fs::create_dir(&entities).unwrap();
    let temporary = temporary_path(&entities.join("r.0.0.mca"));
    fs::write(&temporary, b"unpublished").unwrap();
    fs::set_permissions(&region, fs::Permissions::from_mode(0o000)).unwrap();
    let result = recover_temporaries(directory.path());
    fs::set_permissions(&region, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_ok());
    assert!(!temporary.exists());
}
