//! Positional loot queries while the raid tick owns the mutable raid map.

use super::{
    World,
    raid::{Raid, Raids},
};
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct RaidLootState {
    center: BlockPos,
    active: Arc<AtomicBool>,
}

impl Raids {
    pub(crate) fn has_raid_at_for_loot(&self, pos: &BlockPos) -> bool {
        self.get_raid_at(pos).is_some()
            || self.loot_tick_view.iter().any(|raid| {
                raid.active.load(Ordering::Acquire)
                    && raid.center.to_f64().squared_distance_to_vec(&pos.to_f64())
                        <= Raid::VALID_RAID_RADIUS_SQR
            })
    }
}

struct RaidTick<'a> {
    world: &'a World,
    raids: Raids,
}

impl Drop for RaidTick<'_> {
    fn drop(&mut self) {
        for raid in self.raids.raid_map.values_mut() {
            raid.loot_tick_active = None;
        }
        let mut registry = self
            .world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Preserve raids created by callbacks while the tick owns the original map.
        for (id, raid) in registry.raid_map.drain() {
            self.raids.raid_map.insert(id, raid);
        }
        self.raids.next_id = self.raids.next_id.max(registry.next_id);
        *registry = std::mem::take(&mut self.raids);
    }
}

pub(super) fn tick_raids(world: &Arc<World>) {
    let raids = {
        let mut registry = world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut raids = std::mem::take(&mut *registry);
        registry.loot_tick_view = raids
            .raid_map
            .values_mut()
            .map(|raid| {
                let active = Arc::new(AtomicBool::new(raid.is_active()));
                raid.loot_tick_active = Some(active.clone());
                RaidLootState {
                    center: raid.center,
                    active,
                }
            })
            .collect();
        raids
    };
    // Publish the read view before callbacks; retain no registry guard while ticking.
    let mut owner = RaidTick { world, raids };
    owner.raids.tick(world);
}
