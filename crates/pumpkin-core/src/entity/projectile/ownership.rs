use crate::{
    entity::{Entity, EntityBase},
    world::World,
};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use uuid::Uuid;

struct OwnerCache {
    entity: Option<Weak<dyn EntityBase>>,
    missing_tick: Option<(Uuid, Weak<World>, i64)>,
}

/// Projectile's persistent owner and collision/deflection state.
pub struct ProjectileState {
    owner: Mutex<Option<Uuid>>,
    resolved_owner: Mutex<OwnerCache>,
    #[cfg(test)]
    pub(super) owner_requests: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) owner_lookups: std::sync::atomic::AtomicUsize,
    acceleration: AtomicU64,
    pub(super) left_owner: AtomicBool,
    has_been_shot: AtomicBool,
    left_owner_checked: AtomicBool,
    pub(super) last_deflected_by: Mutex<Option<Uuid>>,
}

impl Default for ProjectileState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ProjectileState {
    #[must_use]
    pub const fn new(owner: Option<Uuid>) -> Self {
        Self {
            owner: Mutex::new(owner),
            resolved_owner: Mutex::new(OwnerCache {
                entity: None,
                missing_tick: None,
            }),
            #[cfg(test)]
            owner_requests: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            owner_lookups: std::sync::atomic::AtomicUsize::new(0),
            acceleration: AtomicU64::new(super::fireball::INITIAL_ACCELERATION_POWER.to_bits()),
            left_owner: AtomicBool::new(false),
            has_been_shot: AtomicBool::new(false),
            left_owner_checked: AtomicBool::new(false),
            last_deflected_by: Mutex::new(None),
        }
    }

    pub fn acceleration_power(&self) -> f64 {
        f64::from_bits(self.acceleration.load(Ordering::Relaxed))
    }
    pub fn set_acceleration_power(&self, power: f64) {
        self.acceleration.store(power.to_bits(), Ordering::Relaxed);
    }

    pub fn from_id(entity: &Entity, owner_id: Option<i32>) -> Self {
        Self::new(
            owner_id
                .and_then(|id| entity.world.load().get_entity_by_id(id))
                .map(|owner| owner.get_entity().entity_uuid),
        )
    }

    /// Resolves the saved UUID in any loaded dimension, retrying misses next tick or after world/owner changes.
    pub fn owner(&self, entity: &Entity) -> Option<Arc<dyn EntityBase>> {
        #[cfg(test)]
        self.owner_requests.fetch_add(1, Ordering::Relaxed);
        let uuid = (*self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))?;
        // EntityReference.getEntity caches until removal; Weak avoids Rust ownership cycles.
        let mut cached = self
            .resolved_owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(owner) = cached.entity.as_ref().and_then(Weak::upgrade)
            && owner.get_entity().entity_uuid == uuid
            && !owner.get_entity().is_removed()
        {
            return Some(owner);
        }
        // EntityReference.getEntity retries unresolved UUIDs; bound Pumpkin's linear scans per tick.
        let world = entity.world.load_full();
        let tick = world.get_world_age();
        if cached
            .missing_tick
            .as_ref()
            .is_some_and(|(missing, previous, age)| {
                *missing == uuid && previous.ptr_eq(&Arc::downgrade(&world)) && *age == tick
            })
        {
            return None;
        }
        #[cfg(test)]
        self.owner_lookups.fetch_add(1, Ordering::Relaxed);
        let owner = resolve_owner(&world, uuid);
        cached.entity = owner.as_ref().map(Arc::downgrade);
        cached.missing_tick = owner
            .is_none()
            .then(|| (uuid, Arc::downgrade(&world), tick));
        owner
    }

    pub fn set_owner(&self, owner: Option<&Entity>) {
        self.set_owner_uuid(owner.map(|owner| owner.entity_uuid));
    }

    pub(crate) fn owner_uuid(&self) -> Option<Uuid> {
        *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn set_owner_uuid(&self, owner: Option<Uuid>) {
        let mut saved = self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *saved != owner {
            *saved = owner;
            let mut cache = self
                .resolved_owner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.entity = None;
            cache.missing_tick = None;
        }
    }

    pub fn owned_by(&self, entity: &Entity) -> bool {
        *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == Some(entity.entity_uuid)
    }

    // Projectile.tick / checkLeftOwner: a collision sweep, never an age-based grace period.
    pub fn tick(&self, entity: &Entity) {
        if !self.has_been_shot.swap(true, Ordering::Relaxed) {
            entity.world.load().emit_game_event(
                pumpkin_data::game_event::GameEvent::ProjectileShoot.name(),
                entity.pos.load(),
            );
        }
        self.check_left_owner(entity);
        self.left_owner_checked.store(false, Ordering::Relaxed);
    }

    // Projectile.checkLeftOwner caches the sweep within a tick, including AbstractArrow's early check.
    pub(super) fn check_left_owner(&self, entity: &Entity) {
        if self.left_owner.load(Ordering::Relaxed)
            || self.left_owner_checked.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let Some(owner) = self.owner(entity) else {
            self.left_owner.store(true, Ordering::Relaxed);
            return;
        };
        let movement = entity.velocity.load();
        let sweep = entity
            .bounding_box
            .load()
            .expand_towards(movement.x, movement.y, movement.z)
            .expand_all(1.0);
        let mut pending = vec![root_vehicle(owner)];
        while let Some(other) = pending.pop() {
            if super::can_hit_entity(&other)
                && sweep.intersects(&other.get_entity().bounding_box.load())
            {
                return;
            }
            pending.extend(
                other
                    .get_entity()
                    .passengers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
        }
        self.left_owner.store(true, Ordering::Relaxed);
    }

    pub fn can_hit(&self, projectile: &Entity, target: &Arc<dyn EntityBase>) -> bool {
        self.can_hit_with_owner(projectile, target, self.owner(projectile).as_ref())
    }

    pub(super) fn can_hit_with_owner(
        &self,
        projectile: &Entity,
        target: &Arc<dyn EntityBase>,
        owner: Option<&Arc<dyn EntityBase>>,
    ) -> bool {
        if projectile.entity_id == target.get_entity().entity_id || !super::can_hit_entity(target) {
            return false;
        }
        self.left_owner.load(Ordering::Relaxed)
            || owner.is_none_or(|owner| {
                root_vehicle(owner.clone()).get_entity().entity_uuid
                    != root_vehicle(target.clone()).get_entity().entity_uuid
            })
    }

    pub(crate) fn write_motion_nbt(&self, entity: &Entity, nbt: &mut NbtCompound) {
        if super::hurting::is_hurting(entity) {
            nbt.put_double("acceleration_power", self.acceleration_power());
        }
    }
    pub(crate) fn read_motion_nbt(&self, entity: &Entity, nbt: &NbtCompound) {
        if super::hurting::is_hurting(entity) {
            self.set_acceleration_power(
                nbt.get_double("acceleration_power")
                    .unwrap_or(super::fireball::INITIAL_ACCELERATION_POWER),
            );
        }
    }

    // Projectile.addAdditionalSaveData / readAdditionalSaveData.
    pub fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Some(owner) = *self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            nbt.put_uuid("Owner", owner);
        }
        nbt.put_bool("LeftOwner", self.left_owner.load(Ordering::Relaxed));
        nbt.put_bool("HasBeenShot", self.has_been_shot.load(Ordering::Relaxed));
    }

    pub fn read_nbt(&self, nbt: &NbtCompound) {
        self.set_owner_uuid(nbt.get_uuid("Owner"));
        let mut cache = self
            .resolved_owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.entity = None;
        cache.missing_tick = None;
        self.left_owner.store(
            nbt.get_bool("LeftOwner").unwrap_or(false),
            Ordering::Relaxed,
        );
        self.has_been_shot.store(
            nbt.get_bool("HasBeenShot").unwrap_or(false),
            Ordering::Relaxed,
        );
    }
}

pub(crate) fn resolve_owner(world: &World, uuid: Uuid) -> Option<Arc<dyn EntityBase>> {
    // EntityReference.getEntity delegates to Level.getEntityInAnyDimension.
    world
        .get_player_by_uuid(uuid)
        .map(|player| player as Arc<dyn EntityBase>)
        .or_else(|| world.get_entity_by_uuid(uuid))
        .or_else(|| {
            let server = world.server.upgrade()?;
            server
                .get_player_by_uuid(uuid)
                .map(|player| player as Arc<dyn EntityBase>)
                .or_else(|| {
                    server
                        .worlds
                        .load()
                        .iter()
                        .find_map(|world| world.get_entity_by_uuid(uuid))
                })
        })
        .filter(|entity| !entity.get_entity().is_removed())
}

fn root_vehicle(mut entity: Arc<dyn EntityBase>) -> Arc<dyn EntityBase> {
    while let Some(vehicle) = entity.get_entity().get_vehicle() {
        entity = vehicle;
    }
    entity
}

/// Transfers a projectile between dimension lists and trackers while preserving its owner/state.
// Entity.teleport / teleportCrossDimension; Pumpkin preserves the existing projectile instance.
pub(crate) fn teleport_projectile(
    entity: &Entity,
    position: pumpkin_util::math::vector3::Vector3<f64>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    destination: &Arc<World>,
) -> bool {
    let source = entity.world.load_full();
    if Arc::ptr_eq(&source, destination) {
        return false;
    }
    let Some(projectile) = source.get_entity_by_id(entity.entity_id) else {
        return false;
    };
    let previous_position = entity.pos.load();
    entity.set_pos(position);
    entity.set_world(destination.clone());
    if !destination.spawn_entity(projectile.clone()) {
        entity.set_world(source);
        entity.set_pos(previous_position);
        return true;
    }
    source
        .spawn_state
        .load()
        .remove_entity(&source, projectile.as_ref());
    source
        .entity_tracker
        .remove_entity(projectile.as_ref(), &source);
    source.entities.rcu(|entities| {
        entities
            .iter()
            .filter(|other| other.get_entity().entity_uuid != entity.entity_uuid)
            .cloned()
            .collect::<Vec<_>>()
    });
    entity.teleport(position, yaw, pitch, destination);
    true
}
