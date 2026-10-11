use std::sync::Arc;

use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};

use crate::entity::{
    ageable::AgeableMob,
    mob::{Mob, interaction::MobInteraction},
    player::Player,
};
use crate::item::item_utils::use_player_item;
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::Hand;
use pumpkin_util::math::vector3::Vector3;

#[cfg(test)]
#[path = "animal_interaction_tests.rs"]
mod interaction_tests;

#[cfg(test)]
#[path = "animal_review_test_support.rs"]
pub(crate) mod review_test_support;

#[cfg(test)]
#[path = "animal_tick_review_tests.rs"]
mod tick_review_tests;

#[cfg(test)]
#[path = "animal_love_review_tests.rs"]
mod love_review_tests;

#[cfg(test)]
#[path = "animal_clock_review_tests.rs"]
mod clock_review_tests;

pub trait Animal: Mob {
    fn is_food(&self, item_stack: &ItemStack) -> bool;

    /// Animals prefer grass, then bright spots.
    fn animal_walk_target_value(&self, pos: &pumpkin_util::math::position::BlockPos) -> f32 {
        let world = self.get_mob_entity().living_entity.entity.world.load();
        if world.get_block(&pos.down()).id == pumpkin_data::Block::GRASS_BLOCK.id {
            return 10.0;
        }
        world.get_light_level_dependent_magic_value(pos) - 0.5
    }

    fn play_eating_sound(&self, sound: Sound) {
        let mob_entity = self.get_mob_entity();
        let entity = &mob_entity.living_entity.entity;
        let world = entity.world.load();
        world.play_sound(sound, SoundCategory::Neutral, &entity.pos.load());
    }

    fn write_animal_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        let mob_entity = self.get_mob_entity();
        let in_love = mob_entity
            .love_ticks
            .load(std::sync::atomic::Ordering::Relaxed);
        nbt.put_int("InLove", in_love);
        if let Some(uuid) = mob_entity.breeder.load() {
            nbt.put_uuid("LoveCause", uuid);
        }
    }

    fn read_animal_nbt(&self, nbt: &pumpkin_nbt::compound::NbtCompound) {
        let mob_entity = self.get_mob_entity();
        let in_love = nbt.get_int("InLove").unwrap_or(0);
        let love_cause = nbt.get_uuid("LoveCause");
        mob_entity.set_love_ticks(in_love, love_cause);
    }

    fn animal_interact(
        &self,
        player: &Arc<Player>,
        item_stack: &mut ItemStack,
        ambient_sound: Sound,
    ) -> bool {
        if interact_with_food(self, player, item_stack, ambient_sound, None) {
            return true;
        }
        self.get_mob_entity().mob_interact(player, item_stack)
    }

    fn animal_interact_with_hand(
        &self,
        player: &Arc<Player>,
        item_stack: &mut ItemStack,
        ambient_sound: Sound,
        hand: Hand,
    ) -> bool {
        let interaction = MobInteraction::new(player, item_stack, hand);
        if interact_with_food(self, player, item_stack, ambient_sound, Some(hand)) {
            return interaction.finish(self, player, item_stack);
        }
        // Leash/unleash belongs to the superclass, outside mobInteract's food result.
        self.get_mob_entity().mob_interact(player, item_stack)
    }
}

fn interact_with_food<A: Animal + ?Sized>(
    animal: &A,
    player: &Arc<Player>,
    item_stack: &mut ItemStack,
    ambient_sound: Sound,
    hand: Option<Hand>,
) -> bool {
    if !animal.is_food(item_stack) {
        return false;
    }
    let mob_entity = animal.get_mob_entity();
    let entity = &mob_entity.living_entity.entity;
    let age = entity.age.load(std::sync::atomic::Ordering::Relaxed);
    // Legacy species still use positive Entity.age as an elapsed clock.
    let adult = if animal.as_ageable().is_some() {
        age == 0
    } else {
        age >= 0
    };
    if adult && mob_entity.is_breeding_ready() && !mob_entity.is_in_love() {
        use_player_item(player, item_stack, hand);
        mob_entity.set_love_ticks(600, Some(player.gameprofile.id));
        // Vanilla Animal.setInLove sends status 18; clients render its seven-heart burst.
        entity.world.load().send_entity_status(
            entity,
            pumpkin_data::entity::EntityStatus::InLoveHearts,
            Some(ActorEventID::InLoveHearts),
        );
        animal.play_eating_sound(ambient_sound);
        return true;
    }

    if age < 0 && animal.as_ageable().is_none_or(AgeableMob::can_age_up) {
        use_player_item(player, item_stack, hand);
        if let Some(ageable) = animal.as_ageable() {
            let seconds = crate::entity::ageable::feeding_speed_up_seconds(-age);
            ageable.age_up(seconds, true);
        } else {
            // Preserve feeding for legacy species without AgeableData. Their
            // full age lifecycle is separate from the ageable feeding contract.
            entity
                .age
                .fetch_add((-age / 10).max(1), std::sync::atomic::Ordering::Relaxed);
        }
        entity.world.load().spawn_particle(
            entity.pos.load() + Vector3::new(0.0, f64::from(entity.height()), 0.0),
            Vector3::new(0.5, 0.5, 0.5),
            1.0,
            7,
            Particle::HappyVillager,
        );
        animal.play_eating_sound(ambient_sound);
        return true;
    }
    false
}

#[must_use]
pub fn get_dye_color_from_item(item: &pumpkin_data::item::Item) -> Option<u8> {
    match item.registry_key {
        "white_dye" => Some(0),
        "orange_dye" => Some(1),
        "magenta_dye" => Some(2),
        "light_blue_dye" => Some(3),
        "yellow_dye" => Some(4),
        "lime_dye" => Some(5),
        "pink_dye" => Some(6),
        "gray_dye" => Some(7),
        "light_gray_dye" => Some(8),
        "cyan_dye" => Some(9),
        "purple_dye" => Some(10),
        "blue_dye" => Some(11),
        "brown_dye" => Some(12),
        "green_dye" => Some(13),
        "red_dye" => Some(14),
        "black_dye" => Some(15),
        _ => None,
    }
}

#[must_use]
pub fn get_carpet_color_from_item(item: &pumpkin_data::item::Item) -> Option<u8> {
    match item.registry_key {
        "white_carpet" => Some(0),
        "orange_carpet" => Some(1),
        "magenta_carpet" => Some(2),
        "light_blue_carpet" => Some(3),
        "yellow_carpet" => Some(4),
        "lime_carpet" => Some(5),
        "pink_carpet" => Some(6),
        "gray_carpet" => Some(7),
        "light_gray_carpet" => Some(8),
        "cyan_carpet" => Some(9),
        "purple_carpet" => Some(10),
        "blue_carpet" => Some(11),
        "brown_carpet" => Some(12),
        "green_carpet" => Some(13),
        "red_carpet" => Some(14),
        "black_carpet" => Some(15),
        _ => None,
    }
}
