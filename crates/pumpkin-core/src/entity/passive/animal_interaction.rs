use std::{sync::Arc, sync::atomic::Ordering};

use pumpkin_data::{
    item_stack::ItemStack,
    particle::Particle,
    sound::{Sound, SoundCategory},
};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::{Hand, math::vector3::Vector3};

use super::animal::Animal;
use crate::entity::{mob::interaction::MobInteraction, player::Player};

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
            let world = entity.world.load();
            let pos = entity.pos.load();

            world.send_entity_status(
                entity,
                pumpkin_data::entity::EntityStatus::InLoveHearts,
                Some(ActorEventID::InLoveHearts),
            );

            world.spawn_particle(
                pos + Vector3::new(0.0, f64::from(entity.height()), 0.0),
                Vector3::new(0.5, 0.5, 0.5),
                1.0,
                7,
                Particle::Heart,
            );
            world.play_sound(ambient_sound, SoundCategory::Neutral, &entity.pos.load());
            if let Some(source) = &source {
                source.refresh(player, item_stack);
            }
            return true;
        }

        if age < 0 {
            if let Some(source) = &source {
                if !source.use_player_item(player, item_stack) {
                    return true;
                }
            } else {
                item_stack.decrement_unless_creative(player.gamemode.load(), 1);
            }
            let speedup = (-age / 10).max(1);
            mob_entity
                .living_entity
                .entity
                .age
                .fetch_add(speedup, Ordering::Relaxed);

            let entity = &mob_entity.living_entity.entity;
            let world = entity.world.load();
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
