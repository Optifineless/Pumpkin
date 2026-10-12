//! Short, leaf-only writes shared with entity persistence commits.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Entity, EntityBase, RemovalReason};
use std::sync::Arc;

/// Level-30 entity admission. Never call gameplay code while holding its guard.
pub struct PersistenceRevision {
    data: Mutex<u64>,
    id: i32,
}

#[cfg(debug_assertions)]
thread_local! { static HELD: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) }; }

pub struct RevisionGuard<'a> {
    data: MutexGuard<'a, u64>,
    id: i32,
}

impl std::ops::Deref for RevisionGuard<'_> {
    type Target = u64;
    fn deref(&self) -> &u64 {
        &self.data
    }
}
impl std::ops::DerefMut for RevisionGuard<'_> {
    fn deref_mut(&mut self) -> &mut u64 {
        &mut self.data
    }
}
impl Drop for RevisionGuard<'_> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        HELD.with(|held| held.borrow_mut().retain(|id| *id != self.id));
        #[cfg(not(debug_assertions))]
        let _ = self.id;
    }
}

impl PersistenceRevision {
    pub const fn new(id: i32) -> Self {
        Self {
            data: Mutex::new(0),
            id,
        }
    }
    fn check_order(&self) {
        #[cfg(debug_assertions)]
        HELD.with(|held| {
            assert!(
                held.borrow().iter().all(|id| *id < self.id),
                "entity persistence admission order/recursion"
            );
        });
    }
    fn guard<'a>(&self, data: MutexGuard<'a, u64>) -> RevisionGuard<'a> {
        #[cfg(debug_assertions)]
        HELD.with(|held| held.borrow_mut().push(self.id));
        RevisionGuard { data, id: self.id }
    }
    pub fn read(&self) -> u64 {
        *self.lock()
    }
    pub fn lock(&self) -> RevisionGuard<'_> {
        self.check_order();
        self.guard(self.data.lock().unwrap_or_else(PoisonError::into_inner))
    }
    pub fn try_lock(&self) -> Option<RevisionGuard<'_>> {
        self.check_order();
        self.data.try_lock().ok().map(|data| self.guard(data))
    }
}

pub fn assert_released() {
    #[cfg(debug_assertions)]
    HELD.with(|held| {
        assert!(
            held.borrow().is_empty(),
            "gameplay boundary under entity persistence admission"
        );
    });
}

impl Entity {
    /// Runs only leaf field writes; callbacks, packets and world queries belong outside.
    pub(crate) fn update_persistent_state(&self, write: impl FnOnce()) -> bool {
        let mut revision = self.persistence_revision.lock();
        if self.removal_reason.load().is_some() {
            return false;
        }
        write();
        *revision = revision.wrapping_add(1);
        true
    }

    /// Entity.setRemoved preserves the first reason, before removal callbacks.
    pub(crate) fn set_removed(&self, reason: RemovalReason) -> bool {
        let mut revision = self.persistence_revision.lock();
        set_removed_fields(&self.removal_reason, &self.removed, reason, &mut revision)
    }

    // Entity.addPassenger: the two links become visible in one persistence transaction.
    pub(crate) fn link_passenger(
        &self,
        vehicle: Arc<dyn EntityBase>,
        passenger: Arc<dyn EntityBase>,
    ) -> bool {
        let linked = passenger.clone();
        let other = linked.get_entity();
        let (first, second) = if self.entity_id < other.entity_id {
            (self, other)
        } else {
            (other, self)
        };
        if std::ptr::eq(first, second) {
            return false;
        }
        let mut a = first.persistence_revision.lock();
        let mut b = second.persistence_revision.lock();
        if self.removal_reason.load().is_some()
            || other.removal_reason.load().is_some()
            || !Arc::ptr_eq(&self.world.load_full(), &other.world.load_full())
        {
            return false;
        }
        *other.vehicle.lock().unwrap_or_else(PoisonError::into_inner) = Some(vehicle);
        self.passengers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(passenger);
        *a = a.wrapping_add(1);
        *b = b.wrapping_add(1);
        true
    }

    // Entity.removePassenger: detach both links before any repositioning or tracker work.
    pub(crate) fn unlink_passenger(&self, id: i32) -> Option<Arc<dyn EntityBase>> {
        let passengers = self
            .passengers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let passenger = passengers
            .iter()
            .find(|passenger| passenger.get_entity().entity_id == id)
            .cloned()?;
        let other = passenger.get_entity();
        let (first, second) = if self.entity_id < other.entity_id {
            (self, other)
        } else {
            (other, self)
        };
        if std::ptr::eq(first, second) {
            return None;
        }
        let mut a = first.persistence_revision.lock();
        let mut b = second.persistence_revision.lock();
        let mut passengers = self
            .passengers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let index = passengers
            .iter()
            .position(|candidate| Arc::ptr_eq(candidate, &passenger))?;
        passengers.remove(index);
        *other.vehicle.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *a = a.wrapping_add(1);
        *b = b.wrapping_add(1);
        Some(passenger.clone())
    }
}

use pumpkin_util::math::get_section_cord;
use pumpkin_util::math::{
    boundingbox::BoundingBox, position::BlockPos, vector2::Vector2, vector3::Vector3,
};

impl Entity {
    // Entity.setPos updates position and section membership before its level callback.
    pub(super) fn set_pos_with_revision(&self, new_position: Vector3<f64>) {
        let pos = self.pos.load();
        if pos != new_position {
            if !self.update_persistent_state(|| self.set_position_fields(new_position)) {
                return;
            }
            self.update_position_biome();
        }
    }
}

impl Entity {
    fn set_position_fields(&self, new_position: Vector3<f64>) {
        self.pos.store(new_position);
        self.bounding_box.store(BoundingBox::new_from_pos(
            new_position.x,
            new_position.y,
            new_position.z,
            &self.entity_dimension.load(),
        ));

        let floor_x = new_position.x.floor() as i32;
        let floor_y = new_position.y.floor() as i32;
        let floor_z = new_position.z.floor() as i32;

        let block_pos = self.block_pos.load();
        let block_pos_vec = block_pos.0;
        if floor_x != block_pos_vec.x || floor_y != block_pos_vec.y || floor_z != block_pos_vec.z {
            let new_block_pos = Vector3::new(floor_x, floor_y, floor_z);
            let new_bp = BlockPos(new_block_pos);
            self.block_pos.store(new_bp);

            let chunk_pos = self.chunk_pos.load();
            if get_section_cord(floor_x) != chunk_pos.x || get_section_cord(floor_z) != chunk_pos.y
            {
                self.chunk_pos.store(Vector2::new(
                    get_section_cord(new_block_pos.x),
                    get_section_cord(new_block_pos.z),
                ));
            }
        }
    }

    /// Publishes projectile position and destination together, before tracker/plugin callbacks.
    pub(crate) fn transfer_projectile_state(
        &self,
        position: Vector3<f64>,
        world: Arc<crate::world::World>,
    ) -> bool {
        let block_pos = BlockPos::floored_v(position);
        let biome = world.level.get_rough_biome(&block_pos);
        self.update_persistent_state(|| {
            self.set_position_fields(position);
            self.current_biome.store(Arc::new(biome));
            self.last_biome_update_pos.store(block_pos);
            self.world.store(world);
        })
    }
}

impl Entity {
    fn update_position_biome(&self) {
        assert_released();
        let block_pos = self.block_pos.load();
        if block_pos != self.last_biome_update_pos.load() {
            let biome = self.world.load().level.get_rough_biome(&block_pos);
            self.current_biome.store(Arc::new(biome));
            self.last_biome_update_pos.store(block_pos);
        }
    }

    // Entity.teleportSameDimension updates position/rotation before sending teleport packets.
    pub(super) fn set_teleport_state(
        &self,
        position: Vector3<f64>,
        yaw: Option<f32>,
        pitch: Option<f32>,
    ) -> bool {
        let changed = self.update_persistent_state(|| {
            self.set_position_fields(position);
            if let Some(yaw) = yaw {
                self.yaw.store(yaw);
            }
            if let Some(pitch) = pitch {
                self.set_pitch(pitch);
            }
        });
        if changed {
            self.update_position_biome();
        }
        changed
    }
}

/// Applies Entity.setRemoved's field transition under caller-held revision admission.
pub fn set_removed_fields(
    removal_reason: &crossbeam::atomic::AtomicCell<Option<RemovalReason>>,
    removed: &std::sync::atomic::AtomicBool,
    reason: RemovalReason,
    revision: &mut u64,
) -> bool {
    if removal_reason.compare_exchange(None, Some(reason)).is_err() {
        return false;
    }
    removed.store(true, std::sync::atomic::Ordering::Release);
    *revision = revision.wrapping_add(1);
    true
}
