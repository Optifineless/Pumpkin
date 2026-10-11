use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::server::Server;
use crate::world::World;

use super::markers::{Phase, Record};
use super::slots::WorkerMarker;
use super::{WatchdogGuard, WorldStats};

impl Server {
    /// Starts tick stall detection explicitly; constructing a server does not start it.
    /// Keep the returned guard until shutdown. Only one watchdog may run per server.
    pub fn start_watchdog(&self) -> std::io::Result<Option<WatchdogGuard>> {
        WatchdogGuard::start(self.watchdog.clone(), self.runtime.metrics())
    }

    pub(crate) fn watchdog_tick_start(&self) {
        self.watchdog.begin_tick();
    }

    /// Registers diagnostic workers before networking or gameplay can start.
    pub(crate) fn watchdog_prepare_workers(&self) {
        if self.watchdog.max_tick_ms > 0 {
            self.watchdog_update_world_stats(&self.worlds.load());
            // Registration precedes plugins and ticker startup, never queued behind gameplay.
            rayon::broadcast(|_| self.watchdog.slots.register());
        }
    }

    pub(crate) fn watchdog_register_ticker(&self) {
        if self.watchdog.enabled.load(Ordering::Relaxed) {
            self.watchdog.slots.register();
        }
    }

    // ServerLevel.getWatchdogStats: keep copied labels and atomic counts. The
    // catalog allocates at startup or world replacement, never on ordinary ticks.
    /// Samples the dispatch slice without reloading the live world list.
    pub(crate) fn watchdog_update_world_stats(&self, worlds: &[Arc<World>]) {
        if self.watchdog.max_tick_ms <= 0 {
            return;
        }
        let previous = self.watchdog.worlds.load();
        if worlds.len() != previous.len()
            || worlds
                .iter()
                .zip(previous.iter())
                .any(|(w, s)| w.uuid != s.id)
        {
            self.watchdog.worlds.store(Arc::new(
                worlds
                    .iter()
                    .map(|world| WorldStats {
                        id: world.uuid,
                        name: world.get_world_name().to_owned(),
                        dimension: world.dimension.minecraft_name,
                        players: AtomicUsize::new(0),
                        chunks: AtomicUsize::new(0),
                        phase: Arc::new(Record::new()),
                    })
                    .collect(),
            ));
        }
        for (world, stats) in worlds.iter().zip(self.watchdog.worlds.load().iter()) {
            stats
                .players
                .store(world.players.load().len(), Ordering::Relaxed);
            stats
                .chunks
                .store(world.level.loaded_chunks.len(), Ordering::Relaxed);
        }
    }

    pub(crate) fn watchdog_world(&self, index: usize, phase: Phase) -> WorkerMarker {
        if !self.watchdog.enabled.load(Ordering::Relaxed) {
            return WorkerMarker::default();
        }
        let record = self
            .watchdog
            .worlds
            .load()
            .get(index)
            .map(|w| w.phase.clone());
        let worker = self.watchdog.slots.scope(index, record);
        phase.enter();
        worker
    }

    pub(crate) fn watchdog_worker(&self, world: usize) -> WorkerMarker {
        if self.watchdog.enabled.load(Ordering::Relaxed) {
            self.watchdog.slots.scope(world, None)
        } else {
            WorkerMarker::default()
        }
    }
}
