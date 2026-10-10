use super::{World, entity_persistence};
use crate::entity::EntityBase;
use pumpkin_util::math::vector2::Vector2;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering},
    time::{Duration, Instant},
};

struct BoundaryAction {
    pos: Option<Vector2<i32>>,
    run: Box<dyn FnOnce(&World) + Send>,
}

// ChunkMap.processUnloads forces the excess over 2,000 even when haveTime is false.
const UNLOAD_QUEUE_LIMIT: usize = 2_000;
const UNLOAD_BUDGET: Duration = Duration::from_millis(5);
const ADMISSION_BATCH_SIZE: usize = 32;

#[cfg(test)]
#[path = "chunk_unload_owner_review_tests.rs"]
mod owner_review_tests;

pub(super) fn entity_address(entity: &Arc<dyn EntityBase>) -> usize {
    Arc::as_ptr(entity).cast::<()>() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn followup_exhausted_drain_forces_only_the_excess_over_two_thousand() {
        let fixture = super::super::spawn_test_support::Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        for _ in 0..2_010 {
            let calls = calls.clone();
            fixture
                .world
                .chunk_unload_requests
                .actions
                .push(BoundaryAction {
                    pos: None,
                    run: Box::new(move |_| {
                        calls.fetch_add(1, Ordering::Relaxed);
                    }),
                });
        }
        fixture.world.drain_chunk_unloads_until(|| true);
        assert_eq!(calls.load(Ordering::Relaxed), 10);
        fixture.world.drain_chunk_unloads_until(|| true);
        assert_eq!(calls.load(Ordering::Relaxed), 10);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn followup_index_overrun_still_processes_one_boundary_action() {
        let fixture = super::super::spawn_test_support::Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let completed = calls.clone();
        fixture
            .world
            .chunk_unload_requests
            .actions
            .push(BoundaryAction {
                pos: None,
                run: Box::new(move |_| {
                    completed.fetch_add(1, Ordering::Relaxed);
                }),
            });
        let checks = AtomicUsize::new(0);
        fixture
            .world
            .drain_chunk_unloads_until(|| checks.fetch_add(1, Ordering::Relaxed) != 0);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn followup_root_index_rejects_a_move_after_admission_closes() {
        use super::super::spawn_test_support::{Fixture, proto, publish};
        use pumpkin_data::{Block, biome::Biome, entity::EntityType};
        use pumpkin_util::math::vector3::Vector3;
        let fixture = Fixture::new();
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
        let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
        terrain.x = 1;
        publish(&fixture.world, terrain);
        let mob = crate::entity::r#type::from_type(
            &EntityType::PIG,
            Vector3::new(1.5, 64.0, 1.5),
            &fixture.world,
            uuid::Uuid::new_v4(),
        );
        fixture.world.entities.store(Arc::new(vec![mob.clone()]));
        let pos = Vector2::new(1, 0);
        let generation = fixture
            .world
            .level
            .close_queued_chunk_admission(pos, 0)
            .unwrap();
        let mut batch = DrainBatch::default();
        batch.generations.insert(pos, generation);
        batch.roots.insert(Vector2::new(0, 0), vec![mob.clone()]);
        *fixture.world.chunk_unload_requests.batch.lock().unwrap() = Some(batch);
        mob.get_entity().set_pos(Vector3::new(17.5, 64.0, 1.5));
        let current = fixture
            .world
            .level
            .chunk_lifecycles
            .at(pos)
            .lock()
            .unwrap()
            .generation;
        assert!(!fixture.world.snapshot_unloading_entities(pos, current));
        assert!(!fixture.world.unloading_entities.contains_key(&pos));
        fixture
            .world
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap()
            .take();
        fixture.finish().await;
    }
}

#[derive(Default)]
struct DrainBatch {
    generations: HashMap<Vector2<i32>, u64>,
    roots: HashMap<Vector2<i32>, Vec<Arc<dyn EntityBase>>>,
    custom: HashMap<Vector2<i32>, Vec<pumpkin_util::math::position::BlockPos>>,
    removed: HashSet<usize>,
}

#[derive(Default)]
pub(super) struct ChunkUnloadRequests {
    terrain: dashmap::DashMap<Vector2<i32>, pumpkin_world::level::SyncChunk>,
    terrain_cursor: AtomicUsize,
    entity_loads: Arc<dashmap::DashSet<Vector2<i32>>>,
    actions: crossbeam::queue::SegQueue<BoundaryAction>,
    batch: Mutex<Option<DrainBatch>>,
    draining: Mutex<()>,
}

impl World {
    pub(super) fn queue_chunk_unload(&self, chunk: &pumpkin_world::level::SyncChunk) {
        self.chunk_unload_requests
            .terrain
            .insert(Vector2::new(chunk.x, chunk.z), chunk.clone());
    }

    pub(super) fn drain_chunk_unloads(&self) {
        #[cfg(test)]
        if self.level.chunk_lifecycles.benchmark_legacy_admission() {
            self.drain_chunk_unloads_legacy();
            return;
        }
        self.drain_chunk_unloads_until(|| false);
    }

    #[cfg(test)]
    fn drain_chunk_unloads_legacy(&self) {
        let requests: Vec<_> = self
            .chunk_unload_requests
            .terrain
            .iter()
            .map(|request| (*request.key(), request.value().clone()))
            .collect();
        for (pos, chunk) in requests {
            if self.level.is_chunk_watched(&pos) || self.level.poll_chunk_unload(&chunk) {
                self.chunk_unload_requests
                    .terrain
                    .remove_if(&pos, |_, current| Arc::ptr_eq(current, &chunk));
            }
        }
        for _ in 0..self.chunk_unload_requests.actions.len() {
            if let Some(action) = self.chunk_unload_requests.actions.pop() {
                (action.run)(self);
            }
        }
    }

    fn drain_chunk_unloads_until(&self, exhausted: impl Fn() -> bool) {
        // ServerChunkCache.tick -> ChunkMap.processUnloads runs before the next live tick phase.
        if self.chunk_unload_requests.terrain.is_empty()
            && self.chunk_unload_requests.actions.is_empty()
        {
            return;
        }
        let Ok(_draining) = self.chunk_unload_requests.draining.try_lock() else {
            return;
        };
        let start = Instant::now();
        let have_time = || !exhausted() && start.elapsed() < UNLOAD_BUDGET;
        let mut forced = (self.chunk_unload_requests.terrain.len()
            + self.chunk_unload_requests.actions.len())
        .saturating_sub(UNLOAD_QUEUE_LIMIT);
        let mut batch = DrainBatch::default();
        let requests = self.index_terrain_requests(forced, &have_time, &mut batch);
        let actions = self.index_boundary_actions(forced, &have_time, &mut batch);
        if !requests.is_empty() || !actions.is_empty() {
            for entity in self.entities.load().iter() {
                batch
                    .roots
                    .entry(entity_persistence::root_chunk(entity))
                    .or_default()
                    .push(entity.clone());
            }
            for custom in &self.custom_block_entity_data {
                batch
                    .custom
                    .entry(custom.key().chunk_position())
                    .or_default()
                    .push(*custom.key());
            }
        }
        *self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(batch);
        // Index construction may consume the allowance; one admitted operation still progresses.
        let mut progressed = false;
        for (pos, chunk) in requests {
            if progressed && forced == 0 && !have_time() {
                self.release_queued_admission(pos);
                continue;
            }
            progressed = true;
            forced = forced.saturating_sub(1);
            if self.level.is_chunk_watched(&pos) || self.level.poll_chunk_unload(&chunk) {
                self.chunk_unload_requests
                    .terrain
                    .remove_if(&pos, |_, current| Arc::ptr_eq(current, &chunk));
            }
        }
        for action in actions {
            if progressed && forced == 0 && !have_time() {
                if let Some(pos) = action.pos {
                    self.release_queued_admission(pos);
                }
                self.chunk_unload_requests.actions.push(action);
                continue;
            }
            progressed = true;
            forced = forced.saturating_sub(1);
            (action.run)(self);
        }
        self.publish_unloaded_members();
    }

    fn index_terrain_requests(
        &self,
        forced: usize,
        have_time: &impl Fn() -> bool,
        batch: &mut DrainBatch,
    ) -> Vec<(Vector2<i32>, pumpkin_world::level::SyncChunk)> {
        let queue = &self.chunk_unload_requests;
        let len = queue.terrain.len();
        let offset = queue.terrain_cursor.load(Ordering::Relaxed) % len.max(1);
        let mut requests = Vec::new();
        // ChunkMap.processUnloads polls a queue: failed requests must not monopolize its head.
        for request in queue
            .terrain
            .iter()
            .skip(offset)
            .chain(queue.terrain.iter().take(offset))
        {
            if requests.len() >= forced + ADMISSION_BATCH_SIZE || (forced == 0 && !have_time()) {
                break;
            }
            queue.terrain_cursor.fetch_add(1, Ordering::Relaxed);
            let pos = *request.key();
            if self.level.is_chunk_watched(&pos) {
                requests.push((pos, request.value().clone()));
                continue;
            }
            if self.level.chunk_unload_save_in_progress(pos) {
                continue;
            }
            let Some(generation) = self.level.close_queued_chunk_admission(pos, 0) else {
                continue;
            };
            batch.generations.insert(pos, generation);
            requests.push((pos, request.value().clone()));
        }
        requests
    }

    fn release_queued_admission(&self, pos: Vector2<i32>) {
        let generation = self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(|batch| batch.generations.get(&pos).copied());
        if let Some(generation) = generation {
            self.level.reopen_queued_chunk_admission(pos, generation);
        }
    }

    pub(super) fn request_unload_entity_storage(&self, pos: Vector2<i32>) {
        let pending = self.chunk_unload_requests.entity_loads.clone();
        if !pending.insert(pos) {
            return;
        }
        // PersistentEntitySectionManager.storeChunkSections requests FRESH storage before retrying.
        // Only enqueue here: get_entity_chunk runs outside the caller's lifecycle mutex.
        let level = self.level.clone();
        self.level.spawn_task(async move {
            tokio::select! {
                () = level.cancel_token.cancelled() => {},
                result = level.get_entity_chunk(pos) => {
                    if let Err(error) = result {
                        tracing::error!(?pos, %error, "Entity chunk remains unloaded");
                    }
                },
            }
            pending.remove(&pos);
        });
    }

    fn publish_unloaded_members(&self) {
        let batch = self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(batch) = batch
            && !batch.removed.is_empty()
        {
            self.entities.rcu(|current| {
                current
                    .iter()
                    .filter(|entity| !batch.removed.contains(&entity_address(entity)))
                    .cloned()
                    .collect::<Vec<_>>()
            });
        }
    }

    fn index_boundary_actions(
        &self,
        forced: usize,
        have_time: &impl Fn() -> bool,
        batch: &mut DrainBatch,
    ) -> Vec<BoundaryAction> {
        let mut actions = Vec::new();
        for _ in 0..self
            .chunk_unload_requests
            .actions
            .len()
            .min(forced + ADMISSION_BATCH_SIZE)
        {
            if forced == 0 && !have_time() {
                break;
            }
            let Some(action) = self.chunk_unload_requests.actions.pop() else {
                break;
            };
            if let Some(pos) = action.pos {
                // Entity-only cleanup retains its own admitted mutation across the boundary wait.
                let Some(generation) = self.level.close_queued_chunk_admission(pos, 1) else {
                    self.chunk_unload_requests.actions.push(action);
                    continue;
                };
                batch.generations.insert(pos, generation);
            }
            actions.push(action);
        }
        actions
    }

    pub(super) fn owns_queued_unload(&self, pos: Vector2<i32>, generation: u64) -> bool {
        self.chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|batch| batch.generations.get(&pos) == Some(&generation))
    }

    pub(super) fn unload_members(
        &self,
        pos: Vector2<i32>,
        generation: u64,
    ) -> Option<Vec<Arc<dyn EntityBase>>> {
        let batch = self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(batch) = batch.as_ref()
            && let Some(indexed) = batch.generations.get(&pos)
        {
            return (*indexed == generation)
                .then(|| batch.roots.get(&pos).cloned().unwrap_or_default());
        }
        Some(
            self.entities
                .load()
                .iter()
                .filter(|entity| entity_persistence::root_chunk(entity) == pos)
                .cloned()
                .collect(),
        )
    }

    pub(super) fn remove_unloaded_members(&self, removed: &[Arc<dyn EntityBase>]) {
        let addresses: HashSet<_> = removed.iter().map(entity_address).collect();
        let mut batch = self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(batch) = batch.as_mut() {
            batch.removed.extend(addresses);
            return;
        }
        drop(batch);
        self.entities.rcu(|current| {
            current
                .iter()
                .filter(|entity| !addresses.contains(&entity_address(entity)))
                .cloned()
                .collect::<Vec<_>>()
        });
    }

    pub(super) fn detach_unloaded_custom_data(&self, pos: Vector2<i32>) {
        let batch = self
            .chunk_unload_requests
            .batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(batch) = batch.as_ref()
            && batch.generations.contains_key(&pos)
        {
            if let Some(positions) = batch.custom.get(&pos) {
                for block in positions {
                    self.custom_block_entity_data.remove(block);
                }
            }
        } else {
            self.custom_block_entity_data
                .retain(|block, _| block.chunk_position() != pos);
        }
    }

    pub(super) async fn at_chunk_tick_boundary<T: Send + 'static>(
        &self,
        pos: Vector2<i32>,
        action: impl FnOnce(&Self) -> T + Send + 'static,
    ) -> Option<T> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.chunk_unload_requests.actions.push(BoundaryAction {
            pos: Some(pos),
            run: Box::new(move |world| {
                if sender.is_closed() {
                    // A cancelled waiter must reopen any admission closed by the index pass.
                    let _cancel = world.level.begin_existing_chunk_mutation(pos);
                } else {
                    let _ = sender.send(action(world));
                }
            }),
        });
        // MinecraftServer.stopServer stops ticks before waiting for outstanding tasks.
        tokio::select! {
            result = receiver => result.ok(),
            () = crate::STOP_INTERRUPT.cancelled() => None,
            () = self.level.cancel_token.cancelled() => None,
        }
    }
}
