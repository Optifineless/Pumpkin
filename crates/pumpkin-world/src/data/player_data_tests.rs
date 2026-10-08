use super::*;

fn inventory(value: i32) -> NbtCompound {
    let mut data = NbtCompound::new();
    data.put_int("InventoryRevision", value);
    data
}

#[test]
fn replaces_complete_file_and_keeps_previous_inode_and_backup() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    let primary = storage.get_player_data_path(&uuid);
    let retained = directory.path().join("previous-inode");
    fs::hard_link(&primary, &retained).unwrap();
    storage.save_player_data(&uuid, inventory(2)).unwrap();
    assert_eq!(
        PlayerDataStorage::read_player_data(&retained)
            .unwrap()
            .unwrap()
            .get_int("InventoryRevision"),
        Some(1)
    );
    assert_eq!(
        PlayerDataStorage::read_player_data(&primary.with_extension("dat_old"))
            .unwrap()
            .unwrap()
            .get_int("InventoryRevision"),
        Some(1)
    );
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(2)
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 3);
}

#[test]
fn failed_replacement_preserves_primary() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    let primary = storage.get_player_data_path(&uuid);
    fs::create_dir(primary.with_extension("dat_old")).unwrap();
    assert!(storage.save_player_data(&uuid, inventory(2)).is_err());
    assert_eq!(
        PlayerDataStorage::read_player_data(&primary)
            .unwrap()
            .unwrap()
            .get_int("InventoryRevision"),
        Some(1)
    );
    assert!(storage.load_player_data(&uuid).is_err());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn stale_periodic_snapshot_cannot_overwrite_disconnect_save() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    let periodic = storage.snapshot(&uuid, || inventory(1));
    let disconnect = storage.snapshot(&uuid, || inventory(2));
    storage.save_snapshot(disconnect).unwrap();
    storage.save_snapshot(periodic).unwrap();
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(2)
    );
}

#[test]
fn backup_recovers_corrupt_and_missing_primary() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    storage.save_player_data(&uuid, inventory(2)).unwrap();
    let primary = storage.get_player_data_path(&uuid);
    fs::write(&primary, b"interrupted gzip").unwrap();
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(1)
    );
    assert!(fs::read_dir(directory.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".corrupt-")
    }));
    fs::remove_file(&primary).unwrap();
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(1)
    );
}

#[test]
fn quarantines_both_bad_files_before_starting_fresh() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    let primary = storage.get_player_data_path(&uuid);
    fs::write(&primary, b"bad primary").unwrap();
    fs::write(primary.with_extension("dat_old"), b"bad backup").unwrap();
    assert!(!storage.load_player_data(&uuid).unwrap().0);
    assert!(!primary.exists());
    let mut contents: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            assert!(entry.file_name().to_string_lossy().contains(".corrupt-"));
            fs::read(entry.path()).unwrap()
        })
        .collect();
    contents.sort();
    assert_eq!(
        contents,
        vec![b"bad backup".to_vec(), b"bad primary".to_vec()]
    );
}

#[tokio::test]
async fn reconnect_waits_for_final_save_and_failure_is_drained() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    let previous_session = storage.acquire_session(&uuid).await.unwrap();
    let mut reconnect = Box::pin(storage.acquire_session(&uuid));
    assert!(
        reconnect
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    let snapshot = storage.snapshot(&uuid, || inventory(2));
    let backup = storage
        .get_player_data_path(&uuid)
        .with_extension("dat_old");
    fs::create_dir(&backup).unwrap();
    assert!(storage.save_snapshot(snapshot).is_err());
    assert!(
        reconnect
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    drop(previous_session);
    let _new_session = reconnect.await.unwrap();
    assert!(storage.load_player_data(&uuid).is_err());
    fs::remove_dir(backup).unwrap();
    storage.flush_all().unwrap();
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(2)
    );
}

#[cfg(unix)]
#[test]
fn failed_recovery_can_be_retried_after_storage_recovers() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    storage.save_player_data(&uuid, inventory(2)).unwrap();
    fs::write(storage.get_player_data_path(&uuid), b"truncated gzip").unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let failure = storage.load_player_data(&uuid);
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failure.is_err());
    assert_eq!(
        storage
            .load_player_data(&uuid)
            .unwrap()
            .1
            .get_int("InventoryRevision"),
        Some(1)
    );
}

#[cfg(unix)]
#[test]
fn io_failures_refuse_recovery_without_quarantining_either_file() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    let primary = storage.get_player_data_path(&uuid);
    let backup = primary.with_extension("dat_old");
    // ELOOP while opening is an I/O error, even with a usable backup.
    storage.save_player_data(&uuid, inventory(1)).unwrap();
    fs::rename(&primary, &backup).unwrap();
    let saved_backup = fs::read(&backup).unwrap();
    symlink(&primary, &primary).unwrap();
    assert!(matches!(
        storage.load_player_data(&uuid),
        Err(PlayerDataError::Io(_))
    ));
    assert!(primary.symlink_metadata().unwrap().is_symlink());
    assert_eq!(fs::read(&backup).unwrap(), saved_backup);
    fs::remove_file(&primary).unwrap();
    // EISDIR is raised during gzip reading, after a successful open.
    fs::create_dir(&primary).unwrap();
    assert!(matches!(
        storage.load_player_data(&uuid),
        Err(PlayerDataError::Io(_))
    ));
    assert!(primary.is_dir());
    assert_eq!(fs::read(&backup).unwrap(), saved_backup);
    // A corrupt primary plus an inaccessible backup must leave both alone too.
    fs::remove_dir(&primary).unwrap();
    fs::write(&primary, b"bad gzip").unwrap();
    fs::remove_file(&backup).unwrap();
    fs::create_dir(&backup).unwrap();
    assert!(matches!(
        storage.load_player_data(&uuid),
        Err(PlayerDataError::Io(_))
    ));
    assert_eq!(fs::read(&primary).unwrap(), b"bad gzip");
    assert!(backup.is_dir());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn session_timeout_refuses_login_without_releasing_the_owner() {
    let directory = tempfile::tempdir().unwrap();
    let storage = PlayerDataStorage::new(directory.path(), true);
    let uuid = Uuid::new_v4();
    let owner = storage.acquire_session(&uuid).await.unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(35),
        storage.acquire_session(&uuid),
    )
    .await
    .unwrap();
    assert!(
        matches!(result, Err(PlayerDataError::Io(error)) if error.kind() == io::ErrorKind::TimedOut)
    );
    assert!(storage.player_state(&uuid).session.try_lock().is_err());
    drop(owner);
    assert!(storage.acquire_session(&uuid).await.is_ok());
}
