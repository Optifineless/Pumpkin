//! Raid membership and loot queries while the tick owns the mutable raid map.

use super::{
    World,
    raid::{Raid, Raids},
};
use crate::entity::{EntityBase, mob::raider::Raider};
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct RaidLootState {
    id: i32,
    center: BlockPos,
    active: Arc<AtomicBool>,
}

pub(super) struct RaidMemberChange {
    raid_id: i32,
    wave: i32,
    uuid: uuid::Uuid,
    leader: bool,
    death: bool,
    killer: Option<uuid::Uuid>,
}

impl RaidMemberChange {
    fn apply(&self, raid: &mut Raid) -> bool {
        if self.death {
            // Raider.die / Raid.removeFromRaid(removeFromTotalHealth=false).
            if self.leader {
                raid.group_to_leader_map.remove(&self.wave);
            }
            if let Some(killer) = self.killer {
                raid.heroes_of_the_village.insert(killer);
            }
            raid.group_raider_map
                .get_mut(&self.wave)
                .is_some_and(|group| group.remove(&self.uuid))
        } else {
            // Raider.readAdditionalSaveData rejoins the saved wave without adding health.
            raid.group_raider_map
                .entry(self.wave)
                .or_default()
                .insert(self.uuid);
            if self.leader {
                raid.group_to_leader_map.insert(self.wave, self.uuid);
            }
            false
        }
    }
}

pub fn load_raider<R: Raider + ?Sized>(raider: &R, raid_id: i32) {
    let raw = raider.get_entity();
    let world = raw.world.load();
    let mut raids = world
        .raids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let exists = raids.contains_for_loot(raid_id);
    raider
        .get_raider_data()
        .raid_id
        .store(exists.then_some(raid_id));
    if exists {
        let change = RaidMemberChange {
            raid_id,
            wave: raider.get_wave(),
            uuid: raw.entity_uuid,
            leader: raider.is_patrol_leader(),
            death: false,
            killer: None,
        };
        if let Some(raid) = raids.get_mut(raid_id) {
            change.apply(raid);
        } else {
            raids.pending_members.push(change);
        }
    }
    drop(raids);
    if exists && raider.is_patrol_leader() {
        // Raider.readAdditionalSaveData calls Raid.setLeader after rejoining.
        let banner = crate::entity::mob::raider::create_ominous_banner();
        let mob = raider.get_mob_entity();
        mob.living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .put(
                &pumpkin_data::data_component_impl::EquipmentSlot::HEAD,
                banner,
            );
        mob.set_guaranteed_drop(&pumpkin_data::data_component_impl::EquipmentSlot::HEAD);
    }
}

pub(super) fn apply_pending_members(raid: &mut Raid, world: &World) {
    let changes: Vec<_> = {
        let mut registry = world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry
            .pending_members
            .extract_if(.., |change| change.raid_id == raid.id)
            .collect()
    };
    let mut removed = false;
    for change in changes {
        removed |= change.apply(raid);
    }
    if removed {
        // The tick's normal bossbar publication follows. Do not introduce another
        // callback between applying membership and the next progression decision.
        snapshot_bossbar(raid, world);
    }
}

pub fn raider_die(entity: &dyn EntityBase, killer: Option<&dyn EntityBase>) {
    let Some(raider) = entity
        .get_mob()
        .and_then(crate::entity::mob::Mob::as_raider)
    else {
        return;
    };
    let Some(raid_id) = raider.get_raider_data().raid_id.swap(None) else {
        return;
    };
    let world = entity.get_entity().world.load_full();
    let update = {
        let mut raids = world
            .raids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let change = RaidMemberChange {
            raid_id,
            wave: raider.get_wave(),
            uuid: entity.get_entity().entity_uuid,
            leader: raider.is_patrol_leader(),
            death: true,
            killer: killer
                .filter(|e| e.get_player().is_some())
                .map(|e| e.get_entity().entity_uuid),
        };
        if let Some(raid) = raids.get_mut(raid_id) {
            change.apply(raid).then(|| snapshot_bossbar(raid, &world))
        } else {
            if raids.contains_for_loot(raid_id) {
                raids.pending_members.push(change);
            }
            None
        }
    };
    if let Some(update) = update {
        send_bossbar(&world, update);
    }
}

fn snapshot_bossbar(raid: &mut Raid, world: &World) -> (super::bossbar::Bossbar, Vec<uuid::Uuid>) {
    let health = raid.get_health_of_living_raiders(world);
    raid.bossbar.health = if raid.total_health > 0.0 {
        (health / raid.total_health).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (
        raid.bossbar.clone(),
        raid.players_in_raid.iter().copied().collect(),
    )
}

fn send_bossbar(world: &World, (bar, recipients): (super::bossbar::Bossbar, Vec<uuid::Uuid>)) {
    let players = world.players.load_full();
    for player in players
        .iter()
        .filter(|p| recipients.contains(&p.gameprofile.id))
    {
        player.update_bossbar_health(&bar.uuid, bar.health);
        player.update_bossbar_title(&bar.uuid, bar.title.clone());
    }
}

impl Raids {
    pub(crate) fn contains_for_loot(&self, id: i32) -> bool {
        self.raid_map.contains_key(&id) || self.loot_tick_view.iter().any(|r| r.id == id)
    }

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
        let mut updates = Vec::new();
        for change in registry.pending_members.drain(..) {
            if let Some(raid) = self.raids.get_mut(change.raid_id)
                && change.apply(raid)
            {
                updates.push(snapshot_bossbar(raid, self.world));
            }
        }
        // Preserve raids created by callbacks while the tick owns the original map.
        for (id, raid) in registry.raid_map.drain() {
            self.raids.raid_map.insert(id, raid);
        }
        self.raids.next_id = self.raids.next_id.max(registry.next_id);
        *registry = std::mem::take(&mut self.raids);
        drop(registry);
        for update in updates {
            send_bossbar(self.world, update);
        }
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
                    id: raid.id,
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
