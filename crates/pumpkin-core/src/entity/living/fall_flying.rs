use super::{LivingEntity, damage_transaction::DamageToken};
use crate::{
    entity::{
        EntityBase,
        death_loot::equipment_slots_in_vanilla_order,
        equipment_damage::{EquippedItem, damage_equipped_item_if},
    },
    plugin::api::events::entity::entity_toggle_glide::EntityToggleGlideEvent,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{EquipmentSlot, EquippableImpl},
    effect::StatusEffect,
    game_event::GameEvent,
    item_stack::ItemStack,
};
use rand::seq::IndexedRandom;
use std::sync::atomic::Ordering::Relaxed;

const GLIDE_EVENT_INTERVAL: i32 = 10;
const GLIDER_DAMAGE_INTERVAL: i32 = 20;

impl LivingEntity {
    // LivingEntity.aiStep/updateFallFlying/tick; player travel remains client authoritative.
    pub(super) fn tick_fall_flying(&self, caller: &dyn EntityBase) {
        if !self.entity.is_fall_flying() {
            self.entity.fall_fly_ticks.store(0, Relaxed);
            return;
        }
        let life = self.damage_owner.enter();
        if !self.flight_owner_current(caller, &life) {
            return;
        }
        self.check_fall_distance_accumulation(caller);
        let check_fall_fly_ticks = self.entity.fall_fly_ticks.load(Relaxed).wrapping_add(1);
        self.entity
            .fall_fly_ticks
            .store(check_fall_fly_ticks, Relaxed);
        if !self.can_glide(caller) {
            self.stop_invalid_glide(caller, &life);
            return;
        }

        if check_fall_fly_ticks % GLIDER_DAMAGE_INTERVAL == 0 {
            // EquipmentSlot.VALUES, not the selected hand or a hardcoded chest item.
            let gliders: Vec<_> = equipment_slots_in_vanilla_order()
                .iter()
                .map(|slot| EquippedItem::capture(caller, slot))
                .filter(|item| Self::can_glide_using(&item.stack, &item.slot))
                .collect();
            if let Some(item) = gliders.choose(&mut rand::rng()) {
                damage_equipped_item_if(caller, item, Some(&life), |stack| {
                    (self.flight_owner_current(caller, &life)
                        && self.entity.is_fall_flying()
                        && self.entity.fall_fly_ticks.load(Relaxed) == check_fall_fly_ticks
                        && (item.slot != EquipmentSlot::MAIN_HAND
                            || caller.get_player().is_none_or(|player| {
                                item.inventory_index
                                    == Some(usize::from(player.inventory.get_selected_slot()))
                            }))
                        && self.can_glide_conditions(caller)
                        && Self::can_glide_using(stack, &item.slot))
                    .then_some(1)
                });
            }
        }
        if !self.flight_owner_current(caller, &life)
            || !self.entity.is_fall_flying()
            || self.entity.fall_fly_ticks.load(Relaxed) != check_fall_fly_ticks
        {
            return;
        }
        if check_fall_fly_ticks % GLIDE_EVENT_INTERVAL == 0 {
            self.entity
                .world
                .load_full()
                .emit_game_event(GameEvent::ElytraGlide.name(), self.entity.pos.load());
        }
        // Item damage/game-event callbacks may remove the glider or change flight validity.
        if self.flight_owner_current(caller, &life)
            && self.entity.is_fall_flying()
            && !self.can_glide(caller)
        {
            self.stop_invalid_glide(caller, &life);
        }
    }

    // Player.tryToStartFallFlying adds current-flight and liquid checks around canGlide.
    pub(crate) fn can_start_fall_flying(&self, caller: &dyn EntityBase) -> bool {
        !self.entity.is_fall_flying()
            && !self.entity.touching_water.load(Relaxed)
            && !self.entity.touching_lava.load(Relaxed)
            && self.can_glide(caller)
    }

    fn can_glide(&self, caller: &dyn EntityBase) -> bool {
        self.can_glide_conditions(caller)
            && self
                .snapshot_equipped_stacks()
                .iter()
                .any(|(slot, stack)| Self::can_glide_using(stack, slot))
    }

    fn check_fall_distance_accumulation(&self, caller: &dyn EntityBase) {
        // Entity.checkFallDistanceAccumulation; accepted client motion replaces player travel.
        let movement = caller.get_player().map_or_else(
            || self.entity.velocity.load(),
            super::super::player::Player::get_known_movement,
        );
        if movement.y > -0.5 && self.fall_distance.load() > 1.0 {
            self.fall_distance.store(1.0);
        }
    }

    fn can_glide_conditions(&self, caller: &dyn EntityBase) -> bool {
        // LivingEntity.canGlide, with Player.canGlide's abilities.flying override.
        !self.entity.on_ground.load(Relaxed)
            && !self.entity.has_vehicle()
            && !self.has_effect(&StatusEffect::LEVITATION)
            && !caller
                .get_player()
                .is_some_and(super::super::player::Player::is_flying)
    }

    fn can_glide_using(stack: &ItemStack, slot: &EquipmentSlot) -> bool {
        // LivingEntity.canGlideUsing -> ItemStack.nextDamageWillBreak.
        !stack.is_empty()
            && stack.has_data_component(DataComponent::Glider)
            && stack
                .get_data_component::<EquippableImpl>()
                .is_some_and(|equippable| {
                    equippable.slot == slot
                        && !(stack.is_damageable()
                            && stack.get_damage() >= stack.get_max_damage().unwrap_or(0) - 1)
                })
    }

    fn flight_owner_current(&self, caller: &dyn EntityBase, life: &DamageToken) -> bool {
        life.is_current_life()
            && !self.entity.is_removed()
            && !self.dead.load(Relaxed)
            && self.health.load() > 0.0
            && caller.get_player().is_none_or(|player| {
                // UUID lookup also finds tracking-only pearl arrivals, not active tick owners.
                !player.client.closed()
                    && self
                        .entity
                        .world
                        .load_full()
                        .contains_player_instance(player)
            })
    }

    fn stop_invalid_glide(&self, caller: &dyn EntityBase, life: &DamageToken) {
        let mut event = EntityToggleGlideEvent::new(self.entity.entity_id, false);
        if let Some(server) = self.entity.world.load_full().server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        // Callbacks can cancel, repair equipment, end flight themselves or replace the life/session.
        if !event.cancelled
            && self.flight_owner_current(caller, life)
            && self.entity.is_fall_flying()
            && !self.can_glide(caller)
            && !event.is_gliding
        {
            self.entity.set_fall_flying(false);
        }
    }
}
