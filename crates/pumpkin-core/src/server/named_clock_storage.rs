//! Fork save-root adaptation of ServerClockManager.init/packState.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use pumpkin_data::dimension::Dimension;
use pumpkin_world::world_info::{
    MAXIMUM_SUPPORTED_WORLD_DATA_VERSION, WorldInfoError,
    data_files::{DimensionClock, WorldClocksData, read_world_clocks, write_world_clocks},
};

use super::Server;
use crate::world::World;

pub(super) struct ClockRoots {
    main: PathBuf,
    retained: Mutex<HashMap<PathBuf, NamedClockState>>,
}

#[derive(Clone)]
struct NamedClockState {
    data: WorldClocksData,
    owners: HashMap<String, uuid::Uuid>,
}

impl ClockRoots {
    pub(super) fn new(main: &Path) -> std::io::Result<Self> {
        Ok(Self {
            main: main.canonicalize()?,
            retained: Mutex::default(),
        })
    }
}

pub fn clock_root_identity(root: &Path) -> Option<PathBuf> {
    match root.canonicalize() {
        Ok(root) => Some(root),
        Err(error) => {
            tracing::warn!("Failed to resolve clock root {}: {error}", root.display());
            None
        }
    }
}

impl Server {
    pub(crate) fn is_main_clock_root(&self, identity: Option<&Path>) -> bool {
        identity == Some(self.clock_roots.main.as_path())
    }

    /// Recheck retention and membership after construction, before native publication.
    pub(super) fn admit_named_clock_world(self: &Arc<Self>, world: Arc<World>) -> Arc<World> {
        let identity = world.clock_root_identity.clone();
        let loaded = identity
            .as_ref()
            .filter(|root| !self.is_main_clock_root(Some(root)))
            .map(|root| read_world_clocks(root));
        let existing = {
            let _save = self
                .world_info_save_lock
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let worlds = self.worlds.load_full();
            let existing = loaded.as_ref().and_then(|_| {
                worlds
                    .iter()
                    .find(|candidate| {
                        candidate.clock_root_identity == identity
                            && candidate.dimension == world.dimension
                            && !candidate.level.cancel_token.is_cancelled()
                    })
                    .cloned()
            });
            if existing.is_none() {
                if let (Some(root), Some(loaded)) = (identity, loaded) {
                    self.restore_named_admission(&world, root, loaded, &worlds);
                }
                self.worlds.rcu(|current| {
                    let mut worlds = (**current).clone();
                    worlds.push(world.clone());
                    worlds
                });
            }
            existing
        };
        if let Some(existing) = existing {
            // A competing admission won; remove the unpublished portal cycle before cleanup.
            let level = world.level.clone();
            level.world_portal.store(Arc::new(None));
            self.spawn_task(async move {
                if let Err(error) = level.shutdown().await {
                    tracing::error!("Failed to stop duplicate clock-root level: {error}");
                }
            });
            existing
        } else {
            world
        }
    }

    fn restore_named_admission(
        &self,
        world: &World,
        root: PathBuf,
        loaded: WorldClocksData,
        worlds: &[Arc<World>],
    ) {
        let mut roots = self
            .clock_roots
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let retained = roots.entry(root).or_insert_with(|| NamedClockState {
            data: loaded,
            owners: HashMap::new(),
        });
        if let Some(key) = world.dimension.default_clock {
            if let Some(previous) = worlds.iter().rev().find(|previous| {
                previous.clock_root_identity == world.clock_root_identity
                    && previous.dimension.default_clock == Some(key)
                    && retained.owners.get(key) == Some(&previous.uuid)
            }) {
                retained
                    .data
                    .clocks
                    .insert(key.into(), super::clock_storage::snapshot_clock(previous));
            }
            let mut time = world
                .level_time
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(clock) = retained.data.clocks.get(key) {
                time.time_of_day = clock.total_ticks;
                time.partial_tick = clock.partial_tick;
                time.rate = clock.rate;
                time.paused = clock.paused;
            }
            retained.data.clocks.insert(
                key.into(),
                DimensionClock {
                    total_ticks: time.time_of_day,
                    partial_tick: time.partial_tick,
                    rate: time.rate,
                    paused: time.paused,
                },
            );
            retained.owners.insert(key.into(), world.uuid);
        }
    }

    pub(super) fn retain_named_unloaded_clock(&self, retired: &Arc<World>, worlds: &[Arc<World>]) {
        let Some(root) = retired
            .clock_root_identity
            .as_ref()
            .filter(|root| !self.is_main_clock_root(Some(root)))
        else {
            return;
        };
        let Some(key) = retired.dimension.default_clock else {
            return;
        };
        let mut roots = self
            .clock_roots
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(retained) = roots.get_mut(root)
            && let Some(owner) = worlds.iter().find(|world| {
                world.clock_root_identity.as_ref() == Some(root)
                    && world.dimension.default_clock == Some(key)
                    && retained.owners.get(key) == Some(&world.uuid)
            })
        {
            retained
                .data
                .clocks
                .insert(key.into(), super::clock_storage::snapshot_clock(owner));
        }
    }

    /// Called under the existing disk-save boundary, with no registry/time/RCU guard across I/O.
    pub(super) fn write_named_clock_roots(
        &self,
        only_root: Option<&Path>,
    ) -> Result<(), WorldInfoError> {
        let roots: Vec<_> = self
            .clock_roots
            .retained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .filter(|root| only_root.is_none_or(|only| root.as_path() == only))
            .cloned()
            .collect();
        let worlds = self.worlds.load_full();
        let mut failures = Vec::new();
        for root in roots {
            // Start from this root's own opaque/unloaded document, never main LevelData.
            let mut data = read_world_clocks(&root);
            let retained = self
                .clock_roots
                .retained
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&root)
                .cloned();
            let Some(retained) = retained else {
                continue;
            };
            data.clocks.extend(retained.data.clocks);
            data.clocks
                .extend(live_root_clocks(&root, &worlds, &retained.owners));
            data.data_version = MAXIMUM_SUPPORTED_WORLD_DATA_VERSION;
            data.migrate_dimension_aliases();
            self.clock_roots
                .retained
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(
                    root.clone(),
                    NamedClockState {
                        data: data.clone(),
                        owners: retained.owners,
                    },
                );
            if let Err(error) = write_world_clocks(&root, &data) {
                failures.push(format!("clock root {}: {error}", root.display()));
            }
        }
        save_result(&failures)
    }

    pub(super) fn save_named_clock_root(&self, root: &Path) -> Result<(), WorldInfoError> {
        let _save = self
            .world_info_save_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.write_named_clock_roots(Some(root))
    }

    pub(crate) async fn save_named_world_clock(
        self: &Arc<Self>,
        world: &World,
    ) -> Result<(), String> {
        let Some(root) = world
            .clock_root_identity
            .as_ref()
            .filter(|root| !self.is_main_clock_root(Some(root)))
            .cloned()
        else {
            return Ok(());
        };
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.save_named_clock_root(&root))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }

    pub(crate) fn schedule_clock_save(self: &Arc<Self>, world: &World) {
        if self.is_main_clock_root(world.clock_root_identity.as_deref()) {
            if world.saves_default_clock
                && world.dimension.minecraft_name == Dimension::OVERWORLD.minecraft_name
            {
                self.schedule_world_info_save();
            }
            return;
        }
        let Some(root) = world.clock_root_identity.clone() else {
            return;
        };
        let server = self.clone();
        self.spawn_task(async move {
            match tokio::task::spawn_blocking(move || server.save_named_clock_root(&root)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!("Failed to save named clock: {error}"),
                Err(error) => tracing::error!("Failed to save named clock: {error}"),
            }
        });
    }
}

fn live_root_clocks(
    root: &Path,
    worlds: &[Arc<World>],
    owners: &HashMap<String, uuid::Uuid>,
) -> HashMap<String, DimensionClock> {
    // UUID ownership survives retirement, so an older cancelled member cannot revive its clock.
    // Final shutdown still snapshots the current owner even after its scheduler is cancelled.
    worlds
        .iter()
        .filter(|world| world.clock_root_identity.as_deref() == Some(root))
        .filter_map(|world| {
            let key = world.dimension.default_clock?;
            (owners.get(key) == Some(&world.uuid))
                .then(|| (key.into(), super::clock_storage::snapshot_clock(world)))
        })
        .collect()
}

pub(super) fn save_result(failures: &[String]) -> Result<(), WorldInfoError> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(WorldInfoError::IoError(std::io::Error::other(
            failures.join("; "),
        )))
    }
}
