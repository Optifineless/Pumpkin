//! ServerClockManager.packState ownership across canonical world unload and metadata retry.

use std::sync::{Arc, PoisonError};

use pumpkin_data::dimension::Dimension;
use pumpkin_world::world_info::{LevelData, data_files::DimensionClock};

use super::Server;
use crate::world::World;

impl Server {
    fn canonical_clock_key(&self, world: &World) -> Option<&'static str> {
        // Plugin-created aliases never own this root's default clock, even for '.' paths.
        (world.saves_default_clock && self.is_main_clock_root(world.clock_root_identity.as_deref()))
            .then_some(world.dimension.default_clock)
            .flatten()
    }

    pub(super) fn retain_unloaded_clock(&self, retired: &Arc<World>) {
        // No ownership is held while shutdown or WorldUnloadEvent callbacks run.
        let _save = self
            .world_info_save_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let worlds = self.worlds.load_full();
        if let Some(key) = self.canonical_clock_key(retired) {
            // A callback may have installed a newer canonical incarnation already.
            let owner = worlds
                .iter()
                .find(|world| {
                    !Arc::ptr_eq(world, retired) && self.canonical_clock_key(world) == Some(key)
                })
                .or_else(|| worlds.iter().find(|world| Arc::ptr_eq(world, retired)));
            if let Some(owner) = owner {
                let clock = snapshot_clock(owner);
                self.level_info.rcu(|previous| {
                    let mut data = (**previous).clone();
                    data.dimension_clocks.insert(key.to_string(), clock.clone());
                    data
                });
            }
        }
        self.retain_named_unloaded_clock(retired, &worlds);
        self.worlds.rcu(|current| {
            let mut remaining = (**current).clone();
            remaining.retain(|world| !Arc::ptr_eq(world, retired));
            remaining
        });
    }

    pub(super) fn level_data_for_save(&self) -> LevelData {
        let mut data = (**self.level_info.load()).clone();
        // Retained records seed the snapshot; legitimate current owners replace them.
        for world in self.worlds.load().iter().filter(|world| {
            world.saves_default_clock
                && self.is_main_clock_root(world.clock_root_identity.as_deref())
        }) {
            let time = world
                .level_time
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(key) = world.dimension.default_clock {
                data.dimension_clocks.insert(
                    key.to_string(),
                    DimensionClock {
                        total_ticks: time.time_of_day,
                        partial_tick: time.partial_tick,
                        rate: time.rate,
                        paused: time.paused,
                    },
                );
            }
            // PrimaryLevelData owns global game time separately from optional clock identity.
            if world.dimension.minecraft_name == Dimension::OVERWORLD.minecraft_name {
                data.game_time = time.world_age;
                data.day_time = time.time_of_day;
                data.day_time_partial_tick = time.partial_tick;
                data.day_time_rate = time.rate;
                data.day_time_paused = time.paused;
            }
        }
        self.weather_data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .apply_to_level_data(&mut data);
        data
    }

    pub(super) fn retain_clock_snapshot(&self, snapshot: &LevelData) {
        // Called under world_info_save_lock before I/O: a failed write keeps the latest clocks.
        self.level_info.rcu(|previous| {
            let mut data = (**previous).clone();
            data.dimension_clocks.clone_from(&snapshot.dimension_clocks);
            data.game_time = snapshot.game_time;
            data.day_time = snapshot.day_time;
            data.day_time_partial_tick = snapshot.day_time_partial_tick;
            data.day_time_rate = snapshot.day_time_rate;
            data.day_time_paused = snapshot.day_time_paused;
            data
        });
    }
}

pub(super) fn snapshot_clock(world: &World) -> DimensionClock {
    let time = world
        .level_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    DimensionClock {
        total_ticks: time.time_of_day,
        partial_tick: time.partial_tick,
        rate: time.rate,
        paused: time.paused,
    }
}
