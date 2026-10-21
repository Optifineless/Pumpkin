//! `ServerLevel` entity insertion, metadata initialization and passenger trees.

use std::sync::Arc;

use rustc_hash::FxHashSet;

use super::World;
use crate::entity::{EntityBase, decoration::leash_knot::LeashKnotEntity, player::Player};

type ExistingMount = (Arc<dyn EntityBase>, Arc<dyn EntityBase>);

#[derive(Clone, Copy)]
enum AppendRejection {
    DuplicateUuid(usize),
    Occupied,
}

#[cfg(test)]
#[path = "projectile_commit_test_support.rs"]
pub mod projectile_test_support;

// Reserve UUIDs before plugin callbacks, without retaining a mutex across their re-entry.
struct SpawnReservation<'a> {
    world: &'a World,
    ids: FxHashSet<uuid::Uuid>,
}

impl Drop for SpawnReservation<'_> {
    fn drop(&mut self) {
        self.world
            .spawn_uuids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|uuid| !self.ids.contains(uuid));
    }
}

impl World {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Preserve the existing owning spawn API"
    )]
    pub fn spawn_entity_non_save(self: &Arc<Self>, entity: Arc<dyn EntityBase>) {
        self.insert_spawned_entity(&entity, true);
    }

    pub(super) fn insert_spawned_entity(
        self: &Arc<Self>,
        entity: &Arc<dyn EntityBase>,
        account_for_spawn: bool,
    ) -> bool {
        self.admit_spawn_tree(entity, account_for_spawn, || true)
    }

    // UUID admission and list publication occur in one ArcSwap compare-and-swap transaction.
    fn insert_entity_batch(&self, members: &[Arc<dyn EntityBase>]) -> bool {
        let Some(reservation) = self.reserve_spawn_uuids(members) else {
            return false;
        };
        self.insert_reserved_entity_batch(&reservation, members, None, &[])
    }

    fn reserve_spawn_uuids(&self, members: &[Arc<dyn EntityBase>]) -> Option<SpawnReservation<'_>> {
        let ids: FxHashSet<_> = members
            .iter()
            .map(|member| member.get_entity().entity_uuid)
            .collect();
        let mut reserved = self
            .spawn_uuids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // ServerLevel.addFreshEntity -> PersistentEntitySectionManager.addEntity / EntityLookup.add.
        let duplicate_uuid = (ids.len() != members.len())
            .then(|| {
                let mut seen = FxHashSet::default();
                members
                    .iter()
                    .map(|member| member.get_entity().entity_uuid)
                    .find(|uuid| !seen.insert(*uuid))
            })
            .flatten()
            .or_else(|| ids.iter().find(|uuid| reserved.contains(uuid)).copied())
            .or_else(|| {
                self.entities
                    .load()
                    .iter()
                    .map(|entity| entity.get_entity().entity_uuid)
                    .find(|uuid| ids.contains(uuid))
            })
            .or_else(|| {
                self.players
                    .load()
                    .iter()
                    .map(|player| player.get_entity().entity_uuid)
                    .find(|uuid| ids.contains(uuid))
            });
        if let Some(member) = duplicate_uuid.and_then(|uuid| {
            members
                .iter()
                .find(|member| member.get_entity().entity_uuid == uuid)
        }) {
            let entity = member.get_entity();
            tracing::warn!(
                entity_type = entity.entity_type.resource_name,
                "UUID of added entity already exists: {}",
                entity.entity_uuid
            );
            return None;
        }
        reserved.extend(ids.iter().copied());
        Some(SpawnReservation { world: self, ids })
    }

    fn insert_reserved_entity_batch(
        &self,
        reservation: &SpawnReservation<'_>,
        members: &[Arc<dyn EntityBase>],
        skip_accounting: Option<uuid::Uuid>,
        existing_mounts: &[ExistingMount],
    ) -> bool {
        for member in members {
            member.init_data_tracker();
        }
        if let Err(reason) = self.append_reserved_entities(reservation, members) {
            Self::report_append_rejection(members, reason);
            return false;
        }
        self.publish_inserted_entities(members, skip_accounting, existing_mounts);
        true
    }

    // The retryable list transaction must never debit stock or publish packets.
    fn append_reserved_entities(
        &self,
        _reservation: &SpawnReservation<'_>,
        members: &[Arc<dyn EntityBase>],
    ) -> Result<(), AppendRejection> {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
        #[cfg(test)]
        let probe = members
            .first()
            .filter(|member| member.projectile_state().is_some())
            .and_then(|_| projectile_test_support::probe(self));
        #[cfg(test)]
        if probe.as_ref().is_some_and(|probe| probe.reject()) {
            return Err(AppendRejection::Occupied);
        }
        let accepted = AtomicBool::new(false);
        let rejected_index = AtomicUsize::new(0);
        let rejected_for_knot = AtomicBool::new(false);
        // LeashFenceKnotEntity.getOrCreateKnot requires one live knot per fence position.
        self.entities.rcu(|current| {
            #[cfg(test)]
            if let Some(probe) = &probe {
                probe.attempt(self);
            }
            let duplicate_uuid = members.iter().position(|member| {
                let uuid = member.get_entity().entity_uuid;
                current
                    .iter()
                    .any(|other| other.get_entity().entity_uuid == uuid)
                    || self
                        .players
                        .load()
                        .iter()
                        .any(|player| player.get_entity().entity_uuid == uuid)
            });
            let duplicate_knot = members.iter().position(|member| {
                let entity = member.get_entity();
                if entity.entity_type != &pumpkin_data::entity::EntityType::LEASH_KNOT {
                    return false;
                }
                let Some(candidate) = member.cast_any().downcast_ref::<LeashKnotEntity>() else {
                    return false;
                };
                current.iter().any(|other| {
                    let other_entity = other.get_entity();
                    other_entity.entity_type == &pumpkin_data::entity::EntityType::LEASH_KNOT
                        && other
                            .cast_any()
                            .downcast_ref::<LeashKnotEntity>()
                            .is_some_and(|existing| {
                                !existing.is_removed_or_removing()
                                    && existing.block_pos() == candidate.block_pos()
                            })
                })
            });
            let duplicate = duplicate_uuid.or(duplicate_knot);
            rejected_for_knot.store(
                duplicate_uuid.is_none() && duplicate_knot.is_some(),
                Relaxed,
            );
            accepted.store(duplicate.is_none(), Relaxed);
            if let Some(index) = duplicate {
                rejected_index.store(index, Relaxed);
                return (**current).clone();
            }
            let mut next = (**current).clone();
            next.extend(members.iter().cloned());
            next
        });
        if !accepted.load(Relaxed) {
            return Err(if rejected_for_knot.load(Relaxed) {
                AppendRejection::Occupied
            } else {
                AppendRejection::DuplicateUuid(rejected_index.load(Relaxed))
            });
        }
        Ok(())
    }

    fn report_append_rejection(members: &[Arc<dyn EntityBase>], reason: AppendRejection) {
        if let AppendRejection::DuplicateUuid(index) = reason {
            let entity = members[index].get_entity();
            tracing::warn!(
                entity_type = entity.entity_type.resource_name,
                "UUID of added entity already exists: {}",
                entity.entity_uuid
            );
        }
    }

    fn publish_inserted_entities(
        &self,
        members: &[Arc<dyn EntityBase>],
        skip_accounting: Option<uuid::Uuid>,
        existing_mounts: &[ExistingMount],
    ) {
        for (rider, mount) in existing_mounts {
            crate::entity::spawn_mount::attach_accepted_chicken(mount, rider.clone());
        }
        for member in members {
            if skip_accounting != Some(member.get_entity().entity_uuid) {
                self.spawn_state.load().add_entity(self, member.as_ref());
            }
            self.entity_tracker.add_entity(member, self);
        }
        for (_, mount) in existing_mounts {
            crate::entity::spawn_mount::publish_passengers(mount);
        }
    }

    // Projectile admission has already finished. Initialize before reserving; commit owns only
    // source validation, the fallible list append and an infallible local storage debit.
    pub(crate) fn insert_admitted_projectile(
        self: &Arc<Self>,
        projectile: Arc<dyn EntityBase>,
        commit: impl FnOnce(&mut dyn FnMut() -> bool) -> bool,
    ) -> bool {
        if !Arc::ptr_eq(self, &projectile.get_entity().world.load_full()) {
            return false;
        }
        projectile.init_data_tracker();
        let members = [projectile];
        let Some(reservation) = self.reserve_spawn_uuids(&members) else {
            return false;
        };
        let mut rejection = None;
        let inserted = commit(
            &mut || match self.append_reserved_entities(&reservation, &members) {
                Ok(()) => true,
                Err(reason) => {
                    rejection = Some(reason);
                    false
                }
            },
        );
        drop(reservation);
        if let Some(reason) = rejection {
            Self::report_append_rejection(&members, reason);
        }
        if inserted {
            self.publish_inserted_entities(&members, None, &[]);
        }
        inserted
    }

    /// Returns `false` when a plugin cancels the [`EntitySpawnEvent`].
    ///
    /// [`EntitySpawnEvent`]: crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Preserve the existing owning spawn API"
    )]
    pub fn spawn_entity(self: &Arc<Self>, entity: Arc<dyn EntityBase>) -> bool {
        self.spawn_entity_with_passengers(&entity)
    }

    /// Inserts a configured riding tree atomically; any spawn or mount cancellation rejects it all.
    pub fn spawn_entity_with_passengers(self: &Arc<Self>, entity: &Arc<dyn EntityBase>) -> bool {
        self.admit_spawn_tree(entity, true, || true)
    }

    /// Runs callbacks before validation and publication; callers may reject stale spawner state.
    pub(crate) fn admit_spawn_tree(
        self: &Arc<Self>,
        original: &Arc<dyn EntityBase>,
        account_for_spawn: bool,
        validate: impl FnOnce() -> bool,
    ) -> bool {
        use crate::entity::spawn_mount;
        let root = spawn_mount::spawn_root(original);
        let entity = &root;
        let mut riding_tree = crate::entity::spawn_mount::UnpublishedRidingTree::new(entity);
        let mut pending = vec![entity.clone()];
        let mut members = Vec::new();
        let mut uuids = FxHashSet::default();
        while let Some(member) = pending.pop() {
            if !uuids.insert(member.get_entity().entity_uuid) {
                let entity = member.get_entity();
                tracing::warn!(
                    entity_type = entity.entity_type.resource_name,
                    "UUID of added entity already exists: {}",
                    entity.entity_uuid
                );
                return false;
            }
            spawn_mount::materialize_pending_riders(&member);
            pending.extend(
                member
                    .get_entity()
                    .passengers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
            members.push(member);
        }
        let Some(reservation) = self.reserve_spawn_uuids(&members) else {
            return false;
        };
        // Preserve plugin cancellation before insertion, with no mount packet from NBT construction.
        for member in &members {
            let mut event = crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent::new(
                member.get_entity().entity_id,
                member.get_entity().entity_type.id.to_string(),
                member.get_entity().pos.load(),
                self.clone(),
            );
            if let Some(server) = self.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled {
                return false;
            }
        }
        for member in &members {
            if let Some(vehicle) = member.get_entity().get_vehicle()
                && !crate::entity::spawn_mount::accept_mount(&vehicle, member)
            {
                return false;
            }
        }
        let existing_mounts: Vec<_> = members
            .iter()
            .filter_map(|member| {
                spawn_mount::take_existing_chicken(member).map(|mount| (member.clone(), mount))
            })
            .collect();
        for (rider, mount) in &existing_mounts {
            if !spawn_mount::available_chicken(mount)
                || !spawn_mount::accept_mount(mount, rider)
                || !spawn_mount::available_chicken(mount)
            {
                return false;
            }
        }
        if !validate()
            || existing_mounts
                .iter()
                .any(|(_, mount)| !spawn_mount::available_chicken(mount))
        {
            return false;
        }
        // NaturalSpawner.afterSpawn accounts for the requested mob; account for companions here.
        let skip = (!account_for_spawn).then_some(original.get_entity().entity_uuid);
        if !self.insert_reserved_entity_batch(&reservation, &members, skip, &existing_mounts) {
            return false;
        }
        riding_tree.keep_links();
        for member in &members {
            crate::entity::spawn_mount::publish_passengers(member);
        }
        true
    }

    /// Fires [`CreatureSpawnEvent`], then spawns the entity; `false` if either event is cancelled.
    ///
    /// [`CreatureSpawnEvent`]: crate::plugin::api::events::entity::creature_spawn::CreatureSpawnEvent
    pub fn spawn_creature(
        self: &Arc<Self>,
        entity: Arc<dyn EntityBase>,
        reason: crate::plugin::api::events::entity::creature_spawn::CreatureSpawnReason,
        player: Option<Arc<Player>>,
    ) -> bool {
        let base = entity.get_entity();
        let mut event = crate::plugin::api::events::entity::creature_spawn::CreatureSpawnEvent::new(
            base.entity_id,
            base.entity_type.resource_name.to_string(),
            base.pos.load(),
            self.clone(),
            reason,
            player,
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return false;
        }
        self.spawn_entity(entity)
    }

    pub fn add_entity_silent(&self, entity: Arc<dyn EntityBase>) {
        self.insert_entity_batch(&[entity]);
    }

    /// Publishes a restored riding tree without rerunning fresh-spawn or mount events.
    pub(super) fn insert_restored_riding_tree(&self, root: &Arc<dyn EntityBase>) -> bool {
        let mut pending = vec![root.clone()];
        let mut members = Vec::new();
        while let Some(member) = pending.pop() {
            pending.extend(
                member
                    .get_entity()
                    .passengers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
            members.push(member);
        }
        if !self.insert_entity_batch(&members) {
            return false;
        }
        for member in &members {
            crate::entity::spawn_mount::publish_passengers(member);
        }
        true
    }
}

#[cfg(test)]
#[path = "uuid_insertion_tests.rs"]
mod tests;
