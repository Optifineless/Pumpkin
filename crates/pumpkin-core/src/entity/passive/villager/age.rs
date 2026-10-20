//! Villager-local `AgeableMob` state; `Entity.age` remains the elapsed tick counter.
use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::{entity::EntityPose, tracked_data};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::bedrock::client::set_actor_data::{
    MetadataValue, SyncedActorDataList, entity_data_flag, entity_data_key,
};
use pumpkin_util::{
    math::{boundingbox::BoundingBox, vector2::Vector2},
    sync::{CaptureIdentity, LockClass, Mutex, OwnLock, SnapshotScope},
};
use pumpkin_world::chunk::ChunkEntityData;

use super::VillagerEntity;
use crate::{
    entity::{Entity, EntityBase, EntityDimensions, ai::brain::memory::types},
    world::{World, entity_tracker::TrackedEntity},
};

const AGE_SCHEMA: &str = "murgicraft:VillagerAgeSchema";
const AGE_VERSION: i32 = 1;
const AGE_KEYS: [&str; 4] = ["Age", "ForcedAge", "AgeLocked", AGE_SCHEMA];
// Villager.BABY_DIMENSIONS, not the generic LivingEntity half-size body.
const BABY_DIMENSIONS: EntityDimensions = EntityDimensions::new(0.49, 0.98, 0.63);

pub(super) enum VillagerAgeLock {}
impl LockClass for VillagerAgeLock {
    const NAME: &'static str = "villager biological age";
    const LEVEL: u8 = 30;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VillagerAgeSnapshot {
    pub age: i32,
    pub forced_age: i32,
    pub locked: bool,
    pub quarantined: bool,
    pub had_schema: bool,
}

#[derive(Default)]
pub(super) struct VillagerAgeState {
    value: VillagerAgeSnapshot,
    retained: Option<NbtCompound>,
    revision: u64,
    pending: bool,
    boundary_pending: bool,
}

#[derive(Default)]
pub(super) struct VillagerAge {
    state: Mutex<VillagerAgeState, VillagerAgeLock>,
    publishing: AtomicBool,
}

impl VillagerAge {
    fn lock(&self) -> std::sync::LockResult<pumpkin_util::sync::MutexGuard<'_, VillagerAgeState>> {
        self.state.lock()
    }
}

struct AgePublisher<'a>(&'a AtomicBool);
impl Drop for AgePublisher<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// Owned observations only: no registry or storage guard reaches age storage or effects.
struct AgeAuthority {
    world: Arc<World>,
    tracked: Option<Arc<TrackedEntity>>,
    storage: Option<Arc<ChunkEntityData>>,
    chunk: Vector2<i32>,
}

impl AgeAuthority {
    fn capture(villager: &VillagerEntity, live: bool) -> Option<Self> {
        let entity = villager.get_entity();
        let world = entity.world.load_full();
        let tracked = world.entity_tracker.get_tracked_entity(entity.entity_id);
        if entity.is_removed()
            || tracked
                .as_ref()
                .is_some_and(|current| !std::ptr::eq(current.entity.get_entity(), entity))
            || (live && tracked.is_none())
        {
            return None;
        }
        let chunk = entity.chunk_pos.load();
        let storage = world.level.get_entity_chunk_sync(&chunk);
        if live
            && storage
                .as_ref()
                .is_some_and(|s| !s.live.load(Ordering::Acquire))
        {
            return None;
        }
        Some(Self {
            world,
            tracked,
            storage,
            chunk,
        })
    }

    fn is_current(&self, entity: &Entity) -> bool {
        if entity.is_removed()
            || entity.chunk_pos.load() != self.chunk
            || !Arc::ptr_eq(&self.world, &entity.world.load_full())
        {
            return false;
        }
        let current = self
            .world
            .entity_tracker
            .get_tracked_entity(entity.entity_id);
        if !match (&self.tracked, current) {
            (Some(old), Some(current)) => Arc::ptr_eq(old, &current),
            (None, None) => true,
            _ => false,
        } {
            return false;
        }
        match (
            &self.storage,
            self.world.level.get_entity_chunk_sync(&self.chunk),
        ) {
            (Some(storage), Some(current)) => {
                Arc::ptr_eq(storage, &current)
                    && (self.tracked.is_none() || current.live.load(Ordering::Acquire))
            }
            (None, None) => true,
            _ => false,
        }
    }
}

impl VillagerEntity {
    /// Returns one coherent biological-age image, independent of elapsed ticks.
    pub fn biological_age(&self) -> super::VillagerAgeSnapshot {
        self.age_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .value
    }

    pub fn is_baby(&self) -> bool {
        self.biological_age().age < 0
    }

    /// Exact age boundary for the later breeding coordinator, without food or HOME policy.
    pub fn is_age_ready(&self) -> bool {
        let age = self.biological_age();
        !age.quarantined && age.age == 0
    }

    /// Sets biological age; quarantined and removed incarnations reject mutation.
    pub fn set_biological_age(&self, age: i32) -> bool {
        self.change_age(Some(age), false)
    }

    pub fn set_baby(&self, baby: bool) -> bool {
        self.set_biological_age(if baby {
            crate::entity::ageable::BABY_START_AGE
        } else {
            0
        })
    }

    pub(super) fn tick_biological_age(&self) {
        self.change_age(None, true);
        let pending = {
            let state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            state.pending || state.boundary_pending
        };
        if pending && let Some(authority) = AgeAuthority::capture(self, true) {
            self.publish_age(&authority);
        }
    }

    fn change_age(&self, requested: Option<i32>, live: bool) -> bool {
        let Some(authority) = AgeAuthority::capture(self, live) else {
            return false;
        };
        let entity = self.get_entity();
        let (revision, boundary) = {
            let mut state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Retirement marks before capturing this same mutex. An admitted writer is
            // included in that capture; writers following the capture observe removal.
            if entity.removal_reason.load().is_some()
                || !Arc::ptr_eq(&authority.world, &entity.world.load_full())
                || state.value.quarantined
                || (live
                    && (self.mob_entity.living_entity.health.load() <= 0.0
                        || self.mob_entity.living_entity.dead.load(Ordering::Relaxed)))
            {
                return false;
            }
            let old = state.value.age;
            let new = requested.unwrap_or_else(|| {
                if old < 0 && !state.value.locked {
                    old + 1
                } else if old > 0 {
                    old - 1
                } else {
                    old
                }
            });
            if old == new {
                return true;
            }
            state.value.age = new;
            state.pending |= (old < 0) != (new < 0);
            state.boundary_pending |= (old < 0) != (new < 0);
            state.revision = state.revision.wrapping_add(1);
            (state.revision, (old < 0) != (new < 0))
        };
        if boundary && self.age_revision_is_current(revision, &authority) {
            self.publish_age(&authority);
        }
        true
    }

    fn age_revision_is_current(&self, revision: u64, authority: &AgeAuthority) -> bool {
        authority.is_current(self.get_entity())
            && self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .revision
                == revision
    }

    pub(super) fn write_age_nbt(&self, nbt: &mut NbtCompound) {
        let (value, retained) = {
            let instance = std::ptr::from_ref(&self.age_state.state).cast::<()>() as usize;
            let _scope = SnapshotScope::with_own_locks(
                CaptureIdentity {
                    kind: "villager biological age",
                    instance,
                    position: None,
                },
                &[OwnLock::new::<VillagerAgeLock>(instance)],
            );
            let state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (state.value, state.retained.clone())
        };
        if let Some(retained) = retained {
            for key in AGE_KEYS {
                nbt.child_tags.remove(key);
                if let Some(tag) = retained.get(key) {
                    nbt.put(key, tag.clone());
                }
            }
        } else {
            nbt.put_int("Age", value.age);
            nbt.put_int("ForcedAge", value.forced_age);
            nbt.put_bool("AgeLocked", value.locked);
            nbt.put_int(AGE_SCHEMA, AGE_VERSION);
        }
    }

    pub(super) fn read_age_nbt(&self, nbt: &NbtCompound) {
        let Some(authority) = AgeAuthority::capture(self, false) else {
            return;
        };
        let malformed = AGE_KEYS.iter().any(|key| {
            nbt.get(key).is_some_and(|tag| {
                if *key == "AgeLocked" {
                    !matches!(tag, NbtTag::Byte(0 | 1))
                } else {
                    !matches!(tag, NbtTag::Int(_))
                }
            })
        });
        let future = nbt.get(AGE_SCHEMA).is_some() && nbt.get_int(AGE_SCHEMA) != Some(AGE_VERSION);
        let quarantined = malformed || future;
        let retained = quarantined.then(|| {
            let mut raw = NbtCompound::new();
            for key in AGE_KEYS {
                if let Some(tag) = nbt.get(key) {
                    raw.put(key, tag.clone());
                }
            }
            raw
        });
        let value = VillagerAgeSnapshot {
            age: if quarantined {
                0
            } else {
                nbt.get_int("Age").unwrap_or(0)
            },
            forced_age: if quarantined {
                0
            } else {
                nbt.get_int("ForcedAge").unwrap_or(0)
            },
            locked: !quarantined && nbt.get_bool("AgeLocked").unwrap_or(false),
            had_schema: nbt.get(AGE_SCHEMA).is_some(),
            quarantined,
        };
        let entity = self.get_entity();
        let revision = {
            let mut state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if entity.removal_reason.load().is_some()
                || !Arc::ptr_eq(&authority.world, &entity.world.load_full())
                || state.value.quarantined
            {
                return;
            }
            state.value = value;
            state.pending = true;
            state.boundary_pending = true;
            state.retained = retained;
            state.revision = state.revision.wrapping_add(1);
            state.revision
        };
        if quarantined {
            tracing::warn!(villager = %self.get_entity().entity_uuid, "Villager age retained without mutation: unsupported schema or malformed tags");
        }
        if self.age_revision_is_current(revision, &authority) {
            self.publish_age(&authority);
        }
    }

    pub(super) fn append_age_spawn_metadata(
        &self,
        metadata: &mut Vec<u8>,
        version: pumpkin_util::version::JavaMinecraftVersion,
    ) -> Option<()> {
        use pumpkin_protocol::java::client::play::Metadata;
        let age = self.biological_age();
        Metadata::new(tracked_data::villager::DATA_BABY_ID, age.age < 0)
            .write(&mut *metadata, &version)
            .ok()?;
        Metadata::new(tracked_data::villager::AGE_LOCKED, age.locked)
            .write(&mut *metadata, &version)
            .ok()?;
        Some(())
    }

    pub(super) fn init_age_tracker(&self) {
        if let Some(authority) = AgeAuthority::capture(self, false) {
            self.publish_age(&authority);
        }
    }

    fn publish_age(&self, authority: &AgeAuthority) {
        if self
            .age_state
            .publishing
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        let _publisher = AgePublisher(&self.age_state.publishing);
        // One bounded reconciliation handles reentrant/concurrent commits; further work
        // stays pending for the next real tick rather than spinning through callbacks.
        if self.publish_age_image(authority) {
            self.publish_age_image(authority);
        }
        self.flush_age_boundary(authority);
    }

    fn flush_age_boundary(&self, authority: &AgeAuthority) {
        if authority.tracked.is_none() || !authority.is_current(self.get_entity()) {
            return;
        }
        let revision = {
            let mut state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.pending || !state.boundary_pending {
                return;
            }
            state.boundary_pending = false;
            state.revision
        };
        if !self.age_revision_is_current(revision, authority)
            || !self.age_boundary_reached(authority, revision)
        {
            self.age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .boundary_pending = true;
        }
    }

    fn publish_age_image(&self, authority: &AgeAuthority) -> bool {
        if !authority.is_current(self.get_entity()) {
            return false;
        }
        let (age, revision) = {
            let state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (state.value, state.revision)
        };
        let entity = self.get_entity();
        if !self.age_revision_is_current(revision, authority) {
            return true;
        }
        self.apply_age_dimensions(Self::dimensions_for_age(
            entity,
            entity.pose.load(),
            age.age < 0,
        ));
        // A native navigator may reenter or retire this owner while dimensions change.
        if !self.age_revision_is_current(revision, authority) {
            return true;
        }
        entity.set_synced_data(tracked_data::villager::DATA_BABY_ID, age.age < 0);
        entity.set_synced_data(tracked_data::villager::AGE_LOCKED, age.locked);
        let _ = entity.bedrock_metadata();
        let mut metadata = SyncedActorDataList::new();
        // Keep other actor flags when publishing the baby bit, including false at growth.
        let mask = 1i64 << entity_data_flag::BABY;
        let flags = if age.age < 0 {
            entity.bedrock_flags.fetch_or(mask, Ordering::Relaxed) | mask
        } else {
            entity.bedrock_flags.fetch_and(!mask, Ordering::Relaxed) & !mask
        };
        metadata.set(entity_data_key::FLAGS, MetadataValue::Int64(flags));
        let dimensions = entity.entity_dimension.load();
        metadata.set(
            entity_data_key::WIDTH,
            MetadataValue::Float(dimensions.width),
        );
        metadata.set(
            entity_data_key::HEIGHT,
            MetadataValue::Float(dimensions.height),
        );
        if authority.tracked.is_some() && authority.is_current(entity) {
            entity.send_bedrock_actor_data(&metadata);
        }
        let mut state = self
            .age_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.pending = state.revision != revision;
        state.pending
    }

    pub(super) fn age_dimensions(&self, pose: EntityPose) -> EntityDimensions {
        Self::dimensions_for_age(self.get_entity(), pose, self.is_baby())
    }

    fn dimensions_for_age(entity: &Entity, pose: EntityPose, baby: bool) -> EntityDimensions {
        if pose == EntityPose::Sleeping {
            Entity::get_entity_dimensions(pose)
        } else if baby {
            BABY_DIMENSIONS
        } else {
            Entity::type_dimensions(entity.entity_type)
        }
    }

    pub(super) fn refresh_age_dimensions(&self, pose: EntityPose) {
        let Some(authority) = AgeAuthority::capture(self, false) else {
            return;
        };
        let entity = self.get_entity();
        let revision = {
            let mut state = self
                .age_state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if entity.removal_reason.load().is_some()
                || entity.pose.load() != pose
                || !Arc::ptr_eq(&authority.world, &entity.world.load_full())
            {
                return;
            }
            state.pending = true;
            state.revision = state.revision.wrapping_add(1);
            state.revision
        };
        if self.age_revision_is_current(revision, &authority) {
            self.publish_age(&authority);
        }
    }

    fn apply_age_dimensions(&self, dimensions: EntityDimensions) {
        let entity = self.get_entity();
        let position = entity.pos.load();
        entity.entity_dimension.store(dimensions);
        entity.bounding_box.store(BoundingBox::new_from_pos(
            position.x,
            position.y,
            position.z,
            &dimensions,
        ));
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_mob_dimensions(dimensions.width, dimensions.height);
    }

    fn age_boundary_reached(&self, authority: &AgeAuthority, revision: u64) -> bool {
        // The goal adapter resolves its schedule from is_baby, so retain Brain/TTL rather
        // than replacing it. Stop only the obsolete path/social eligibility and recheck jobs.
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop();
        if !self.age_revision_is_current(revision, authority) {
            return false;
        }
        self.mob_entity
            .brain
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .erase(types::INTERACTION_TARGET.id());
        self.update_job_site(&authority.world);
        // Pumpkin has no findFreePosition growth adjustment or boat hasEnoughSpaceFor
        // facility. Keep its current collision/dismount behavior; do not invent a search.
        true
    }
}

impl VillagerEntity {
    /// Pose changes originate on Entity; route only villagers to the concrete age owner.
    pub(crate) fn pose_dimensions(entity: &Entity, pose: EntityPose) -> Option<EntityDimensions> {
        if entity.entity_type != &pumpkin_data::entity::EntityType::VILLAGER {
            return None;
        }
        let world = entity.world.load();
        if entity.is_removed() {
            return None;
        }
        let Some(current) = world.entity_tracker.get_tracked_entity(entity.entity_id) else {
            // Before admission, the baby flag is the setter's committed projection.
            // Tracker initialization refreshes navigation before the entity is published.
            return Some(Self::dimensions_for_age(
                entity,
                pose,
                entity.bedrock_flags.load(Ordering::Relaxed) & (1i64 << entity_data_flag::BABY)
                    != 0,
            ));
        };
        if !std::ptr::eq(current.entity.get_entity(), entity) {
            return None;
        }
        current.entity.get_mob()?.mob_age_dimensions(pose)
    }

    pub(crate) fn pose_changed(entity: &Entity, pose: EntityPose) -> bool {
        if entity.entity_type != &pumpkin_data::entity::EntityType::VILLAGER {
            return true;
        }
        let world = entity.world.load_full();
        let Some(current) = world.entity_tracker.get_tracked_entity(entity.entity_id) else {
            return !entity.is_removed();
        };
        if !std::ptr::eq(current.entity.get_entity(), entity) {
            return false;
        }
        let Some(villager) = current.entity.cast_any().downcast_ref::<Self>() else {
            return false;
        };
        let Some(authority) = AgeAuthority::capture(villager, false) else {
            return false;
        };
        if let Some(mob) = current.entity.get_mob() {
            mob.mob_refresh_age_dimensions(pose);
        }
        authority.is_current(entity)
    }

    pub(crate) fn sound_baby(entity: &Entity) -> bool {
        let world = entity.world.load();
        world
            .entity_tracker
            .get_tracked_entity(entity.entity_id)
            .is_some_and(|current| {
                std::ptr::eq(current.entity.get_entity(), entity)
                    && !entity.is_removed()
                    && current.entity.is_semantic_baby()
            })
    }
}
