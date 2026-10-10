//! PersistentEntitySectionManager.Callback.onMove/onRemove ownership in a concurrent server.
use std::sync::{Arc, Mutex, atomic::AtomicU64, atomic::AtomicUsize, atomic::Ordering};

use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::level::{
    Level,
    chunk_lifecycle::{ChunkAdmissionCache, ChunkMutation},
};

use super::{Entity, riding_admission::RidingAdmission};

// Keep admission call sites small in the frozen Entity module without moving upstream methods.
macro_rules! admit_mutation {
    ($entity:expr) => {
        match $entity.try_begin_mutation() {
            Some(permit) => permit,
            None => return,
        }
    };
}
pub(super) use admit_mutation;

#[derive(Default)]
pub(super) struct WorldMembership {
    generation: AtomicU64,
    writer: Mutex<()>,
}

// PersistentEntitySectionManager.Callback.onMove retains its current section between moves.
pub(super) struct CachedAdmission {
    pos: Vector2<i32>,
    level: std::sync::Weak<Level>,
    cell: Arc<ChunkAdmissionCache>,
    previous: Option<(Vector2<i32>, Arc<ChunkAdmissionCache>)>,
}

/// Owns entity state/membership admission, including its riding root and a move destination.
///
/// Outside tick scopes, keep through asynchronous effects; identity admission follows chunk moves.
/// Tick-scoped permits must end before the tick read barrier and cannot cross an await.
/// Raw field writes require this same admission.
/// The permit borrows its entity, which must remain alive until the admitted effects finish.
#[must_use]
pub struct EntityMutation<'a> {
    single: Option<ChunkMutation>,
    destination: Option<ChunkMutation>,
    permits: Vec<ChunkMutation>,
    active: Option<&'a AtomicUsize>,
}

impl EntityMutation<'_> {
    fn release_permits(&mut self) {
        drop(self.single.take());
        drop(self.destination.take());
        self.permits.clear();
    }
}

impl Drop for EntityMutation<'_> {
    fn drop(&mut self) {
        self.release_permits();
        if let Some(active) = self.active {
            active.fetch_sub(1, Ordering::Release);
        }
    }
}

/// Owns admission when an asynchronous caller transfers its entity handle with the permit.
/// Tick-scoped permits still must end before the read barrier and cannot cross an await.
#[must_use]
pub struct OwnedEntityMutation {
    mutation: EntityMutation<'static>,
    active: Option<Arc<AtomicUsize>>,
}

impl Drop for OwnedEntityMutation {
    fn drop(&mut self) {
        self.mutation.release_permits();
        if let Some(active) = &self.active {
            active.fetch_sub(1, Ordering::Release);
        }
    }
}

impl Entity {
    // Entity.setLevel: serialize world writers; odd generations reject a store in progress.
    pub(super) fn store_world(&self, world: Arc<crate::world::World>) {
        let _writer = self
            .world_membership
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.world_membership
            .generation
            .fetch_add(1, Ordering::SeqCst);
        self.world.store(world);
        self.world_membership
            .generation
            .fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn has_cached_chunk_admission(&self, level: &Arc<Level>, pos: Vector2<i32>) -> bool {
        self.mutation_chunk.load().as_ref().is_some_and(|cache| {
            std::ptr::eq(cache.level.as_ptr(), Arc::as_ptr(level))
                && cache.pos == pos
                && cache.cell.load().is_some()
        })
    }
    /// Includes admitted writers that started before this entity changed chunks or worlds.
    pub(crate) fn has_admitted_mutations(&self) -> bool {
        self.mutation_admissions.load(Ordering::Acquire) != 0
    }

    /// Admits a state change and invalidates any earlier snapshot; refuses removed handles.
    pub fn try_begin_mutation(&self) -> Option<EntityMutation<'_>> {
        self.try_begin_move_mutation(self.chunk_pos.load())
    }

    /// Retains identity admission for callers moving the entity handle into asynchronous work.
    /// Ordinary synchronous setters should borrow admission through `try_begin_mutation`.
    pub fn try_begin_owned_mutation(&self) -> Option<OwnedEntityMutation> {
        let mut mutation = self.try_begin_mutation()?;
        let active = mutation
            .active
            .take()
            .map(|_| self.mutation_admissions.clone());
        Some(OwnedEntityMutation {
            mutation: EntityMutation {
                single: mutation.single.take(),
                destination: mutation.destination.take(),
                permits: std::mem::take(&mut mutation.permits),
                active: None,
            },
            active,
        })
    }

    pub(super) fn try_begin_position_mutation(
        &self,
        position: Vector3<f64>,
    ) -> Option<EntityMutation<'_>> {
        self.try_begin_move_mutation(BlockPos::floored_v(position).chunk_position())
    }

    fn try_begin_move_mutation(&self, destination: Vector2<i32>) -> Option<EntityMutation<'_>> {
        self.try_begin_move_mutation_with(destination, || {})
    }

    fn try_begin_move_mutation_with(
        &self,
        destination: Vector2<i32>,
        mut before_revalidation: impl FnMut(),
    ) -> Option<EntityMutation<'_>> {
        if self.is_removed() {
            return None;
        }
        let mut world_generation = self.world_membership.generation.load(Ordering::SeqCst);
        let mut world = self.world.load();
        #[cfg(test)]
        if world.level.chunk_lifecycles.benchmark_admission_disabled() {
            return Some(EntityMutation {
                single: None,
                destination: None,
                permits: Vec::new(),
                active: None,
            });
        }
        // Entity.load may change position before readAdditionalSaveData or an async follow-up.
        // A source world's tick barrier cannot protect this identity after a cross-world move.
        self.mutation_admissions.fetch_add(1, Ordering::AcqRel);
        let mut mutation = EntityMutation {
            single: None,
            destination: None,
            permits: Vec::new(),
            active: Some(self.mutation_admissions.as_ref()),
        };
        loop {
            let source = self.chunk_pos.load();
            // Entity.startRiding/removeVehicle: detect an intervening mount, including a round trip.
            let riding = self.riding_admission.load();
            if (source == destination || !world.level.chunk_lifecycles.benchmark_legacy_admission())
                && RidingAdmission::is_unmounted(riding)
            {
                // Callback.onMove changes sections; retain both sides until the move finishes.
                let mut cache = self.mutation_chunk.load();
                let permit = self.admit_cached_chunk(&world.level, source, &mut cache);
                let destination_permit = (source != destination)
                    .then(|| self.admit_cached_chunk(&world.level, destination, &mut cache));
                before_revalidation();
                if self.is_removed() {
                    return None;
                }
                if world_generation.is_multiple_of(2)
                    && self.world_membership.generation.load(Ordering::SeqCst) == world_generation
                    && self.chunk_pos.load() == source
                    && self.riding_admission.load() == riding
                {
                    mutation.single = Some(permit);
                    mutation.destination = destination_permit;
                    return Some(mutation);
                }
                drop(permit);
                drop(destination_permit);
                world_generation = self.world_membership.generation.load(Ordering::SeqCst);
                world = self.world.load();
                continue;
            }
            let mut positions = self.mutation_positions(destination);
            positions.sort_unstable_by_key(|pos| (pos.x, pos.y));
            positions.dedup();
            // Counters, not held mutexes: no waiting lock order across chunks or plugin callbacks.
            let permits = positions
                .iter()
                .map(|pos| world.level.begin_chunk_mutation(*pos))
                .collect();
            if self.is_removed() {
                return None;
            }
            if !world_generation.is_multiple_of(2)
                || self.world_membership.generation.load(Ordering::SeqCst) != world_generation
            {
                drop(permits);
                world_generation = self.world_membership.generation.load(Ordering::SeqCst);
                world = self.world.load();
                continue;
            }
            if self
                .mutation_positions(destination)
                .iter()
                .all(|pos| positions.contains(pos))
            {
                mutation.permits = permits;
                return Some(mutation);
            }
            // Another admitted move changed membership while we collected its source/root chunks.
            drop(permits);
            world_generation = self.world_membership.generation.load(Ordering::SeqCst);
            world = self.world.load();
        }
    }

    fn admit_cached_chunk(
        &self,
        level: &Arc<Level>,
        pos: Vector2<i32>,
        cache: &mut arc_swap::Guard<Option<Arc<CachedAdmission>>>,
    ) -> ChunkMutation {
        if level.chunk_lifecycles.benchmark_legacy_admission() {
            return level.begin_chunk_mutation(pos);
        }
        if let Some(cache) = cache.as_ref()
            && std::ptr::eq(cache.level.as_ptr(), Arc::as_ptr(level))
        {
            if cache.pos == pos
                && let Some(permit) = level.admit_cached_chunk_mutation(&cache.cell)
            {
                return permit;
            }
            if let Some((previous, cached)) = &cache.previous
                && *previous == pos
                && let Some(permit) = level.admit_cached_chunk_mutation(cached)
            {
                return permit;
            }
        }
        let cell = level.chunk_lifecycles.at(pos);
        let permit = level.admit_chunk_mutation(&cell);
        // Border oscillation reuses two cells; quiescence releases their canonical ownership.
        let previous = cache.as_ref().and_then(|cache| {
            std::ptr::eq(cache.level.as_ptr(), Arc::as_ptr(level))
                .then(|| {
                    if cache.pos == pos {
                        cache.previous.clone()
                    } else {
                        Some((cache.pos, cache.cell.clone()))
                    }
                })
                .flatten()
        });
        let cached = Arc::new(CachedAdmission {
            pos,
            level: Arc::downgrade(level),
            cell: cell.cache(),
            previous,
        });
        self.mutation_chunk.store(Some(cached.clone()));
        *cache = arc_swap::Guard::from_inner(Some(cached));
        permit
    }

    fn mutation_positions(&self, destination: Vector2<i32>) -> Vec<Vector2<i32>> {
        let mut positions = vec![self.chunk_pos.load(), destination];
        let mut vehicle = self.get_vehicle();
        while let Some(root) = vehicle {
            positions.push(root.get_entity().chunk_pos.load());
            vehicle = root.get_entity().get_vehicle();
        }
        // Entity.removePassenger can make any descendant a saved root in its own chunk.
        let mut passengers = self
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        while let Some(passenger) = passengers.pop() {
            let entity = passenger.get_entity();
            positions.push(entity.chunk_pos.load());
            passengers.extend(
                entity
                    .passengers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
        }
        positions
    }
}

#[cfg(test)]
#[path = "section_membership_tests.rs"]
mod tests;
