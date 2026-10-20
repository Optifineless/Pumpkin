//! `AbstractBoat`'s base tick, vehicle damage and single-item destruction admission.
use std::sync::{Arc, atomic::Ordering};

use pumpkin_data::{
    damage::DamageType, data_component_impl::CustomNameImpl, item::Item, item_stack::ItemStack,
};
use pumpkin_protocol::{codec::var_int::VarInt, java::client::play::Metadata};
use pumpkin_util::{
    GameMode,
    sync::{LockClass, Mutex},
};

use super::BoatEntity;
use crate::{
    entity::EntityBase,
    plugin::vehicle::{
        vehicle_damage::VehicleDamageEvent, vehicle_destroy::VehicleDestroyEvent,
        vehicle_update::VehicleUpdateEvent,
    },
    world::{BlockEntityMutationPermit, World},
};

pub(super) enum BoatDamageOwner {}

impl LockClass for BoatDamageOwner {
    const NAME: &'static str = "boat damage";
    const LEVEL: u8 = 30;
}

// A logical receipt only: no mutex guard survives the admission statement or any callback.
struct BoatDamageOperation<'a>(&'a Mutex<bool, BoatDamageOwner>);

impl Drop for BoatDamageOperation<'_> {
    fn drop(&mut self) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
    }
}

impl BoatEntity {
    pub(super) fn tick_boat_base(&self, caller: &dyn EntityBase, world: &Arc<World>) {
        // Entity.tick/baseTick, staged locally because a fluid callback can retire this boat.
        // The ordinary base tick would continue into fire metadata after that callback.
        let entity = &self.vehicle.entity;
        let was_in_powder_snow = entity.is_in_powder_snow.load(Ordering::Relaxed);
        entity
            .was_in_powder_snow
            .store(was_in_powder_snow, Ordering::Relaxed);
        entity.is_in_powder_snow.store(false, Ordering::Relaxed);
        entity.update_last_pos();
        entity.tick_portal(caller);
        if !self.is_current_boat(world) {
            return;
        }
        // Entity.baseTick updates currents once; inside-block passes own fluid effects.
        entity.update_fluid_interaction(caller);
        if !self.is_current_boat(world) {
            return;
        }
        entity.check_out_of_world(caller);
        if !self.is_current_boat(world) {
            return;
        }
        let fire_ticks = entity.fire_ticks.load(Ordering::Relaxed);
        let immune = entity.is_fire_immune();
        if fire_ticks > 0 {
            if immune {
                entity.fire_ticks.store(fire_ticks - 4, Ordering::Relaxed);
                if entity.fire_ticks.load(Ordering::Relaxed) < 0 {
                    entity.extinguish();
                }
            } else {
                if fire_ticks % 20 == 0 && !entity.touching_lava.load(Ordering::SeqCst) {
                    caller.damage(caller, 1.0, DamageType::ON_FIRE);
                    if !self.is_current_boat(world) {
                        return;
                    }
                }
                entity.fire_ticks.store(fire_ticks - 1, Ordering::Relaxed);
            }
        }
        if !self.is_current_boat(world) {
            return;
        }
        entity.set_on_fire(entity.fire_ticks.load(Ordering::Relaxed) > 0 && !immune);
        if !self.is_current_boat(world) {
            return;
        }
        let riding_cooldown = entity.riding_cooldown.load(Ordering::Relaxed);
        if riding_cooldown > 0 {
            entity
                .riding_cooldown
                .store(riding_cooldown - 1, Ordering::Relaxed);
        }
    }

    pub(super) fn is_current_boat(&self, world: &Arc<World>) -> bool {
        let entity = &self.vehicle.entity;
        entity.is_alive()
            && !world.level.cancel_token.is_cancelled()
            && Arc::ptr_eq(&entity.world.load_full(), world)
            && world
                .get_entity_by_id(entity.entity_id)
                .is_some_and(|current| std::ptr::eq(current.get_entity(), entity))
    }

    fn boat_commit_permit<'w>(
        &self,
        world: &'w Arc<World>,
    ) -> Option<BlockEntityMutationPermit<'w>> {
        let entity = &self.vehicle.entity;
        let expected = world.get_entity_by_id(entity.entity_id)?;
        if !entity.is_alive()
            || !Arc::ptr_eq(&entity.world.load_full(), world)
            || !std::ptr::eq(expected.get_entity(), entity)
        {
            return None;
        }
        // Entity retirement uses this existing capture gate before withdrawing the registry.
        // Hold it only over leaf state/first-removal commits, never over their effects.
        let permit = world.reserve_block_entity_mutation(self.vehicle.entity.block_pos.load())?;
        if !entity.is_alive()
            || !Arc::ptr_eq(&entity.world.load_full(), world)
            || !world
                .entities
                .load()
                .iter()
                .any(|current| Arc::ptr_eq(current, &expected))
        {
            return None;
        }
        Some(permit)
    }

    fn send_current_boat_wobble(&self, world: &Arc<World>) -> bool {
        if !self.is_current_boat(world) {
            return false;
        }
        self.vehicle.entity.send_meta_data(
            &[
                Metadata::new(
                    pumpkin_data::tracked_data::boat::ID_HURT,
                    VarInt(self.vehicle.get_hurt_time()),
                ),
                Metadata::new(
                    pumpkin_data::tracked_data::boat::ID_HURTDIR,
                    VarInt(self.vehicle.get_hurt_dir()),
                ),
            ],
            None,
        );
        if !self.is_current_boat(world) {
            return false;
        }
        self.vehicle.entity.send_meta_data(
            &[Metadata::new(
                pumpkin_data::tracked_data::boat::ID_DAMAGE,
                self.vehicle.get_damage(),
            )],
            None,
        );
        self.is_current_boat(world)
    }

    pub(super) fn tick_boat_vehicle(&self, world: &Arc<World>) {
        let Some(permit) = self.boat_commit_permit(world) else {
            return;
        };
        let wobble_ticked = {
            let _state = self
                .damage_operation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.vehicle.tick_wobble()
        };
        drop(permit);
        if wobble_ticked && !self.send_current_boat_wobble(world) {
            return;
        }
        if !self.is_current_boat(world) {
            return;
        }
        let mut event = VehicleUpdateEvent::new(self.vehicle.entity.entity_id);
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if self.is_current_boat(world)
            && self
                .vehicle
                .entity
                .velocity_dirty
                .swap(false, Ordering::SeqCst)
        {
            self.vehicle.entity.send_velocity();
        }
    }

    pub(super) fn damage_boat(
        &self,
        amount: f32,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if !self.vehicle.entity.is_alive() {
            return true;
        }
        let world = self.vehicle.entity.world.load_full();
        let _operation = {
            let mut active = self
                .damage_operation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *active {
                return false;
            }
            *active = true;
            BoatDamageOperation(&self.damage_operation)
        };
        if !self.is_current_boat(&world)
            || self
                .vehicle
                .entity
                .is_invulnerable_to_base(&damage_type, cause)
        {
            return false;
        }
        self.damage_admitted_boat(&world, amount, source, cause)
    }

    fn damage_admitted_boat(
        &self,
        world: &Arc<World>,
        amount: f32,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if !self.is_current_boat(world) {
            return false;
        }
        let attacker_id = source.map(|source| source.get_entity().entity_id);
        if !self.is_current_boat(world) {
            return false;
        }
        let mut damage_event =
            VehicleDamageEvent::new(self.vehicle.entity.entity_id, amount, attacker_id);
        if let Some(server) = world.server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut damage_event);
        }
        if damage_event.cancelled || !self.is_current_boat(world) {
            return false;
        }
        let Some(permit) = self.boat_commit_permit(world) else {
            return false;
        };
        let strength = {
            let _state = self
                .damage_operation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.vehicle.apply_damage_wobble_state(amount)
        };
        drop(permit);
        if !self.send_current_boat_wobble(world) {
            return false;
        }

        // VehicleEntity.hurtServer checks the causing player's instabuild ability.
        let creative = cause
            .and_then(EntityBase::get_player)
            .is_some_and(|player| player.gamemode.load() == GameMode::Creative);
        if !self.is_current_boat(world) {
            return false;
        }
        let lethal = creative || strength > 40.0;
        if !lethal {
            return true;
        }
        let mut destroy_event =
            VehicleDestroyEvent::new(self.vehicle.entity.entity_id, attacker_id);
        if let Some(server) = world.server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut destroy_event);
        }
        if destroy_event.cancelled || !self.is_current_boat(world) {
            return false;
        }
        self.destroy_current_boat(world, creative)
    }

    fn destroy_current_boat(&self, world: &Arc<World>, creative: bool) -> bool {
        // AbstractBoat.getDropItem uses the concrete boat/raft/chest variant.
        let Some(item) = Item::from_registry_key(self.vehicle.entity.entity_type.resource_name)
        else {
            return false;
        };
        let mut stack = ItemStack::new(1, item);
        if let Some(name) = self.vehicle.entity.custom_name.load().as_ref().as_ref() {
            stack.set_data_component(CustomNameImpl { name: name.clone() });
        }
        let drop_item = !creative && world.level_info.load().game_rules.entity_drops;
        world.destroy_boat_with_drop(self, drop_item.then_some(stack))
    }
}
