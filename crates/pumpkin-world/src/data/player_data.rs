use pumpkin_nbt::compound::NbtCompound;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions, create_dir_all},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::storage::{safe_replace_file, sync_parent, temporary_path};

pub struct PlayerDataStorage {
    data_path: PathBuf,
    save_enabled: bool,
    players: Mutex<HashMap<Uuid, Arc<SaveState>>>,
}

#[derive(Default)]
struct SaveState {
    sequence: AtomicU64,
    capture: Mutex<()>,
    disk: Mutex<CommitState>,
    pending: Mutex<Option<Arc<PlayerDataSnapshot>>>,
    session: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct CommitState {
    committed: u64,
    blocked: bool,
}

/// An ordered player snapshot captured before dispatching background disk work.
#[derive(Clone)]
pub struct PlayerDataSnapshot {
    uuid: Uuid,
    sequence: u64,
    data: NbtCompound,
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerDataError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("NBT error: {0}")]
    Nbt(String),
}

impl PlayerDataStorage {
    pub fn new(data_path: impl Into<PathBuf>, enabled: bool) -> Self {
        let path = data_path.into();
        if let Err(e) = create_dir_all(&path) {
            error!(
                "Failed to create player data directory at {}: {e}",
                path.display()
            );
        }
        Self {
            data_path: path,
            save_enabled: enabled,
            players: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub const fn get_data_path(&self) -> &PathBuf {
        &self.data_path
    }

    #[must_use]
    pub const fn is_save_enabled(&self) -> bool {
        self.save_enabled
    }

    pub const fn set_save_enabled(&mut self, enabled: bool) {
        self.save_enabled = enabled;
    }

    #[must_use]
    pub fn get_player_data_path(&self, uuid: &Uuid) -> PathBuf {
        self.data_path.join(format!("{uuid}.dat"))
    }

    fn player_state(&self, uuid: &Uuid) -> Arc<SaveState> {
        self.players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(*uuid)
            .or_default()
            .clone()
    }

    /// Captures data under the UUID's save lock and assigns its sequence before scheduling it.
    pub fn snapshot(
        &self,
        uuid: &Uuid,
        capture: impl FnOnce() -> NbtCompound,
    ) -> PlayerDataSnapshot {
        let state = self.player_state(uuid);
        // Snapshot capture never waits for background disk I/O.
        let _capture = state
            .capture
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = capture();
        let sequence = state.sequence.fetch_add(1, Ordering::Release) + 1;
        let snapshot = PlayerDataSnapshot {
            uuid: *uuid,
            sequence,
            data,
        };
        if self.save_enabled {
            *state
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(Arc::new(snapshot.clone()));
        }
        snapshot
    }

    /// Publishes the latest retained snapshot for this UUID, superseding older queued work.
    pub fn save_snapshot(&self, snapshot: PlayerDataSnapshot) -> Result<(), PlayerDataError> {
        let uuid = snapshot.uuid;
        drop(snapshot);
        self.flush_player(&uuid)
    }

    /// Waits up to 30 seconds for the previous session to finish saving and release its UUID.
    pub async fn acquire_session(
        &self,
        uuid: &Uuid,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, PlayerDataError> {
        // Pumpkin's concurrent equivalent of PlayerList.remove must not strand a login.
        const SESSION_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
        tokio::time::timeout(
            SESSION_WAIT,
            self.player_state(uuid).session.clone().lock_owned(),
        )
        .await
        .map_err(|_| {
            error!("Timed out waiting for player storage session {uuid}");
            io::Error::new(io::ErrorKind::TimedOut, "Player storage session is busy").into()
        })
    }

    /// Waits for disk writes and retries all retained snapshots captured before the drain.
    pub fn flush_all(&self) -> Result<(), PlayerDataError> {
        let ids: Vec<_> = self
            .players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect();
        let mut failure = None;
        for uuid in ids {
            if let Err(error) = self.flush_player(&uuid) {
                error!("Failed to drain player {uuid}: {error}");
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn flush_player(&self, uuid: &Uuid) -> Result<(), PlayerDataError> {
        if !self.save_enabled {
            return Ok(());
        }
        let state = self.player_state(uuid);
        let mut disk = state
            .disk
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if disk.blocked {
            return Err(io::Error::other("Player data recovery failed; saving is blocked").into());
        }
        let snapshot = state
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(snapshot) = snapshot {
            self.write_player_data(uuid, snapshot.data.clone())?;
            disk.committed = snapshot.sequence;
            let mut pending = state
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending
                .as_ref()
                .is_some_and(|current| current.sequence == snapshot.sequence)
            {
                *pending = None;
            }
        }
        Ok(())
    }

    pub fn save_player_data(&self, uuid: &Uuid, data: NbtCompound) -> Result<(), PlayerDataError> {
        self.save_snapshot(self.snapshot(uuid, || data))
    }

    // PlayerDataStorage.save: finish and force a unique gzip file before Util.safeReplaceFile.
    fn write_player_data(&self, uuid: &Uuid, data: NbtCompound) -> Result<(), PlayerDataError> {
        create_dir_all(&self.data_path)?;
        let path = self.get_player_data_path(uuid);
        let temporary = temporary_path(&path);
        let result = (|| {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            pumpkin_nbt::nbt_compress::write_gzip_compound_tag(data, &file)
                .map_err(|e| PlayerDataError::Nbt(e.to_string()))?;
            file.sync_all()?;
            drop(file);
            safe_replace_file(&path, &temporary, &path.with_extension("dat_old"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    fn read_player_data(path: &Path) -> Result<Option<NbtCompound>, PlayerDataError> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Self::decode_player_data(file).map(Some)
    }

    fn decode_player_data(
        reader: impl io::Read + io::Seek,
    ) -> Result<NbtCompound, PlayerDataError> {
        // flate2 reports invalid gzip headers/checksums as InvalidInput; raw OS
        // errors (including EINVAL) must still refuse login without quarantine.
        pumpkin_nbt::nbt_compress::read_gzip_compound_tag(reader).map_err(|error| match error {
            pumpkin_nbt::Error::Incomplete(error)
                if error.raw_os_error().is_some()
                    || !matches!(
                        error.kind(),
                        io::ErrorKind::InvalidData
                            | io::ErrorKind::InvalidInput
                            | io::ErrorKind::UnexpectedEof
                    ) =>
            {
                PlayerDataError::Io(error)
            }
            error => PlayerDataError::Nbt(error.to_string()),
        })
    }

    fn quarantine(path: &Path) -> Result<(), PlayerDataError> {
        if !path.try_exists()? {
            return Ok(());
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".corrupt-{timestamp}-{}", Uuid::new_v4()));
        let quarantined = PathBuf::from(name);
        fs::rename(path, &quarantined)?;
        sync_parent(path)?;
        error!(
            "Quarantined unreadable player data: {}",
            quarantined.display()
        );
        Ok(())
    }

    /// Loads the primary or backup; fresh data is allowed only after corrupt files are quarantined.
    pub fn load_player_data(&self, uuid: &Uuid) -> Result<(bool, NbtCompound), PlayerDataError> {
        if !self.save_enabled {
            return Ok((false, NbtCompound::new()));
        }
        let state = self.player_state(uuid);
        {
            let mut disk = state
                .disk
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if disk.blocked {
                self.load_with_recovery(uuid)?;
                disk.blocked = false;
            }
        }
        // A failed disconnect must be retried before any old inventory can be loaded.
        self.flush_player(uuid)?;
        let mut disk = state
            .disk
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Prevent later saves if any part of recovery fails.
        disk.blocked = true;
        let result = self.load_with_recovery(uuid);
        if result.is_ok() {
            disk.blocked = false;
        }
        result
    }

    // PlayerDataStorage.load: try .dat_old after a missing or unreadable primary.
    fn load_with_recovery(&self, uuid: &Uuid) -> Result<(bool, NbtCompound), PlayerDataError> {
        let path = self.get_player_data_path(uuid);
        // PlayerDataStorage.load fallback, narrowed to corruption: an I/O outage
        // refuses this login without renaming either file or rolling back inventory.
        let corrupt_primary = match Self::read_player_data(&path) {
            Ok(Some(data)) => return Ok((true, data)),
            Ok(None) => false,
            Err(error @ PlayerDataError::Nbt(_)) => {
                error!("Failed to decode player data for {uuid}: {error}");
                true
            }
            Err(error) => return Err(error),
        };
        let backup = path.with_extension("dat_old");
        let corrupt_backup = match Self::read_player_data(&backup) {
            Ok(Some(data)) => {
                // Preserve the good backup instead of rotating the damaged primary over it.
                if corrupt_primary {
                    Self::quarantine(&path)?;
                }
                self.write_player_data(uuid, data.clone())?;
                warn!("Recovered player data for {uuid} from .dat_old");
                return Ok((true, data));
            }
            Ok(None) => false,
            Err(error @ PlayerDataError::Nbt(_)) => {
                error!("Failed to decode backup player data for {uuid}: {error}");
                true
            }
            Err(error) => return Err(error),
        };
        if corrupt_primary {
            Self::quarantine(&path)?;
        }
        if corrupt_backup {
            Self::quarantine(&backup)?;
        }
        debug!("Starting fresh player data for {uuid}");
        Ok((false, NbtCompound::new()))
    }
}

#[cfg(test)]
#[path = "player_data_tests.rs"]
mod tests;
