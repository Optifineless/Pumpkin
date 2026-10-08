use crate::{
    enchantment::EnchantmentHelper,
    entity::{
        Entity, EntityBase,
        player::Player,
        projectile::{apply_on_projectile_spawned, fishing_bobber::FishingBobberEntity},
    },
    item::{ItemBehaviour, ItemMetadata},
};
use pumpkin_data::{
    data_component_impl::EquipmentSlot,
    entity::EntityType,
    game_event::GameEvent,
    item::Item,
    sound::{Sound, SoundCategory},
    statistic::StatisticCategory,
};
use pumpkin_util::Hand;
use std::{
    any::Any,
    sync::{Arc, atomic::Ordering::Relaxed},
};

pub struct FishingRodItem;
#[cfg(test)]
#[path = "fishing_rod_tests.rs"]
mod tests;

impl ItemMetadata for FishingRodItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::FISHING_ROD.id])
    }
}

impl ItemBehaviour for FishingRodItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_rotation(item, player, yaw, pitch);
    }

    fn normal_use_with_rotation(&self, item: &Item, player: &Player, yaw: f32, pitch: f32) {
        let hand = if player.inventory().held_item().item == item {
            Hand::Right
        } else {
            Hand::Left
        };
        self.normal_use_with_hand(item, player, yaw, pitch, hand);
    }

    // FishingRodItem.use retrieves with the used hand's rod, otherwise casts along the use rotation.
    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: Hand,
    ) {
        let world = player.world();
        let rod = player.inventory().get_stack_in_hand(hand);
        let id = player.fishing_bobber.load(Relaxed);
        let bobber = world
            .get_entity_by_id(id)
            .filter(|entity| !entity.get_entity().is_removed());
        if let Some(bobber) = bobber {
            if let Some(hook) = bobber.cast_any().downcast_ref::<FishingBobberEntity>() {
                let damage = hook.reel_in(player, &rod, hand);
                if !hook.get_entity().is_removed() {
                    return;
                }
                let slot = match hand {
                    Hand::Right => EquipmentSlot::MAIN_HAND,
                    Hand::Left => EquipmentSlot::OFF_HAND,
                };
                if damage > 0 {
                    player.damage_item_in_slot_if(&slot, |stack| {
                        stack.are_items_and_components_equal(&rod).then_some(damage)
                    });
                }
            }
            world.play_sound_fine(
                Sound::EntityFishingBobberRetrieve,
                SoundCategory::Neutral,
                &player.position(),
                1.0,
                bobber_sound_pitch(),
            );
            world.emit_game_event(GameEvent::ItemInteractFinish.name(), player.position());
            return;
        }
        player.fishing_bobber.store(-1, Relaxed);
        let luck = EnchantmentHelper::modify_fishing_luck_bonus(&rod, 0.0) as i32;
        let lure = (EnchantmentHelper::modify_fishing_time_reduction(&rod, 0.0) * 20.0) as i32;
        let hook = Arc::new(FishingBobberEntity::new_with_rotation(
            Entity::new(
                world.clone(),
                player.position(),
                &EntityType::FISHING_BOBBER,
            ),
            player,
            yaw,
            pitch,
            luck,
            lure,
            hand,
        ));
        if hook.fire_cast_event(hand).is_none() {
            return;
        }
        world.play_sound_fine(
            Sound::EntityFishingBobberThrow,
            SoundCategory::Neutral,
            &player.position(),
            0.5,
            bobber_sound_pitch(),
        );
        player
            .fishing_bobber
            .store(hook.get_entity().entity_id, Relaxed);
        apply_on_projectile_spawned(hook.get_entity(), &rod, None, None);
        if !world.spawn_entity(hook.clone()) {
            hook.clear_owner();
        }
        player.increment_stat(StatisticCategory::Used, i32::from(Item::FISHING_ROD.id), 1);
        world.emit_game_event(GameEvent::ItemInteractStart.name(), player.position());
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// FishingRodItem.use randomizes throw/retrieve pitch identically.
fn bobber_sound_pitch() -> f32 {
    0.4 / (rand::random::<f32>() * 0.4 + 0.8)
}
