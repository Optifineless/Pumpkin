//! The server's saved scoreboard, loaded once before worlds and players.

use std::{path::Path, sync::Arc};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_world::world_info::{MAXIMUM_SUPPORTED_WORLD_DATA_VERSION, WorldInfoError, scoreboard};

use super::Server;
use crate::world::{
    owner_mutex::OwnerMutex,
    scoreboard::{Scoreboard, SharedScoreboard},
};

pub(super) struct ScoreboardStorage {
    writable: bool,
    // Only save tasks take this gate; gameplay never acquires it. Vanilla's
    // SavedData.setData also avoids writing when a dirty notification changed nothing.
    last_saved: Arc<tokio::sync::Mutex<NbtCompound>>,
}

pub(super) fn load_scoreboard(path: &Path) -> (SharedScoreboard, ScoreboardStorage) {
    let loaded = scoreboard::read_scoreboard(path).and_then(|root| {
        let Some(root) = root else {
            return Ok(Scoreboard::default());
        };
        let version = root.get("DataVersion").map_or(Ok(0), |tag| {
            tag.extract_int().ok_or_else(|| {
                WorldInfoError::DeserializationError(
                    "Scoreboard DataVersion must be an integer".into(),
                )
            })
        })?;
        if version > MAXIMUM_SUPPORTED_WORLD_DATA_VERSION {
            return Err(WorldInfoError::UnsupportedDataVersion(version));
        }
        if version < MAXIMUM_SUPPORTED_WORLD_DATA_VERSION {
            tracing::warn!(
                "Loading scoreboard DataVersion {version} without data fixing (server {MAXIMUM_SUPPORTED_WORLD_DATA_VERSION})"
            );
        }
        let data = root.get_compound("data").ok_or_else(|| {
            WorldInfoError::DeserializationError("Missing scoreboard data compound".into())
        })?;
        Scoreboard::from_save_data(data)
    });
    let writable = loaded.is_ok();
    let state = loaded.unwrap_or_else(|error| {
        tracing::error!(
            "Failed to load scoreboard.dat: {error}; scoreboard saves disabled to preserve the original file"
        );
        Scoreboard::default()
    });
    let last_saved = state.to_save_data();
    (
        Arc::new(OwnerMutex::new(state)),
        ScoreboardStorage {
            writable,
            last_saved: Arc::new(tokio::sync::Mutex::new(last_saved)),
        },
    )
}

impl Server {
    pub(crate) async fn save_scoreboard(&self) -> Result<(), String> {
        if !self.scoreboard_storage.writable {
            return Err("scoreboard.dat was not loaded; preserving the original file".into());
        }
        let path = self.basic_config.get_world_path();
        let state = self.scoreboard.clone();
        // The blocking worker retains the save gate through publication, even if its
        // async waiter is cancelled. A later save cannot publish an older snapshot.
        let gate = self
            .scoreboard_storage
            .last_saved
            .clone()
            .lock_owned()
            .await;
        tokio::task::spawn_blocking(move || {
            let mut last_saved = gate;
            let (snapshot, revision) = {
                let scoreboard = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !scoreboard.is_dirty() {
                    return Ok(());
                }
                (scoreboard.clone(), scoreboard.revision())
            };
            let data = snapshot.to_save_data();
            if data != *last_saved {
                scoreboard::write_scoreboard(
                    &path,
                    data.clone(),
                    MAXIMUM_SUPPORTED_WORLD_DATA_VERSION,
                )
                .map_err(|error| error.to_string())?;
                *last_saved = data;
            }
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .mark_saved(revision);
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
    }
}
