use std::{sync::Arc, sync::atomic::Ordering};

use pumpkin_data::{
    data_component_impl::UseRemainderImpl, item_stack::ItemStack, particle::Particle, sound::Sound,
};
use pumpkin_inventory::{Inventory, screen_handler::InventoryPlayer};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::{Hand, math::vector3::Vector3};

use super::animal::Animal;
use crate::entity::{ageable::AgeableMob, mob::interaction::MobInteraction, player::Player};
use crate::item::item_utils;

// Handless Mob callers own their supplied stack; packet callers commit a retained hand.
pub(super) struct StackInteraction(Option<MobInteraction>);

impl StackInteraction {
    pub(super) fn new(player: &Player, hand: Option<Hand>, stack: &ItemStack) -> Self {
        Self(hand.map(|hand| MobInteraction::new(player, hand, stack)))
    }

    pub(super) fn consume(&self, player: &Player, stack: &mut ItemStack) -> bool {
        if let Some(source) = &self.0 {
            return source.consume(player, stack);
        }
        stack.decrement_unless_creative(player.gamemode.load(), 1);
        true
    }

    pub(super) fn use_player_item(&self, player: &Player, stack: &mut ItemStack) -> bool {
        if let Some(source) = &self.0 {
            return source.use_player_item(player, stack);
        }
        if !player.has_infinite_materials()
            && let Some(remainder) = stack.get_data_component::<UseRemainderImpl>()
            && let Some(remainder) = remainder.create()
        {
            return self.create_filled_result(player, stack, remainder, false);
        }
        self.consume(player, stack)
    }

    pub(super) fn create_filled_result(
        &self,
        player: &Player,
        stack: &mut ItemStack,
        mut output: ItemStack,
        limit_creative_stack_size: bool,
    ) -> bool {
        if let Some(source) = &self.0 {
            return source.create_filled_result(player, stack, output, limit_creative_stack_size);
        }
        // ItemUtils.createFilledResult, exchanging the caller's stack directly.
        if limit_creative_stack_size && player.has_infinite_materials() {
            let inventory = player.inventory();
            if !(0..inventory.size()).any(|slot| {
                inventory
                    .get_stack(slot)
                    .are_items_and_components_equal(&output)
            }) {
                inventory.insert_stack_anywhere(&mut output);
            }
        } else {
            stack.decrement_unless_creative(player.gamemode.load(), 1);
            if stack.is_empty() {
                *stack = output;
            } else {
                item_utils::give_or_drop(player, output);
            }
        }
        true
    }

    pub(super) fn refresh(&self, player: &Player, stack: &mut ItemStack) {
        if let Some(source) = &self.0 {
            source.refresh(player, stack);
        }
    }
}

// Animal.mobInteract, retaining the existing age clock and effects for legacy callers.
pub(super) fn interact<A: Animal + ?Sized>(
    animal: &A,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    ambient_sound: Sound,
    hand: Option<Hand>,
) -> bool {
    let source = hand.map(|hand| MobInteraction::new(player, hand, item_stack));
    let mob_entity = animal.get_mob_entity();
    if animal.is_food(item_stack) {
        let age = mob_entity.living_entity.entity.age.load(Ordering::Relaxed);

        if age >= 0 && mob_entity.is_breeding_ready() && !mob_entity.is_in_love() {
            if let Some(source) = &source {
                if !source.use_player_item(player, item_stack) {
                    return true;
                }
            } else {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            }

            mob_entity.set_love_ticks(600, Some(player.gameprofile.id));
            let entity = &mob_entity.living_entity.entity;
            let world = entity.world.load_full();

            world.send_entity_status(
                entity,
                pumpkin_data::entity::EntityStatus::InLoveHearts,
                Some(ActorEventID::InLoveHearts),
            );

            animal.play_eating_sound(ambient_sound);
            if let Some(source) = &source {
                source.refresh(player, item_stack);
            }
            return true;
        }

        let ageable = animal.as_ageable();
        if age < 0 && ageable.is_none_or(AgeableMob::can_age_up) {
            if let Some(source) = &source {
                if !source.use_player_item(player, item_stack) {
                    return true;
                }
            } else {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            }
            if let Some(ageable) = ageable {
                ageable.age_up(get_speed_up_seconds_when_feeding(-age), true);
            } else {
                // Legacy species retain their existing age ownership until #370.
                let speedup = (-age / 10).max(1);
                mob_entity
                    .living_entity
                    .entity
                    .age
                    .fetch_add(speedup, Ordering::Relaxed);
            }

            let entity = &mob_entity.living_entity.entity;
            let world = entity.world.load_full();
            let pos = entity.pos.load();

            world.spawn_particle(
                pos + Vector3::new(0.0, f64::from(entity.height()), 0.0),
                Vector3::new(0.5, 0.5, 0.5),
                1.0,
                7,
                Particle::HappyVillager,
            );
            animal.play_eating_sound(ambient_sound);
            if let Some(source) = &source {
                source.refresh(player, item_stack);
            }
            return true;
        }
    }

    mob_entity.mob_interact(animal, player, item_stack)
}

// AgeableMob.getSpeedUpSecondsWhenFeeding; the trait's static helper is Sized-only.
fn get_speed_up_seconds_when_feeding(ticks_until_adult: i32) -> i32 {
    ((ticks_until_adult / 20) as f32 * 0.1) as i32
}
