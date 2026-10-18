use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::{
    lingering_potion::LingeringPotionEntity, splash_potion::SplashPotionEntity,
};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

pub struct PotionItem;
pub struct SplashPotionItem;
pub struct LingeringPotionItem;

impl ItemMetadata for PotionItem {
    fn ids() -> Box<[u16]> {
        [Item::POTION.id].into()
    }
}

impl ItemMetadata for SplashPotionItem {
    fn ids() -> Box<[u16]> {
        [Item::SPLASH_POTION.id].into()
    }
}

impl ItemMetadata for LingeringPotionItem {
    fn ids() -> Box<[u16]> {
        [Item::LINGERING_POTION.id].into()
    }
}

const POWER: f32 = 0.5;

impl ItemBehaviour for PotionItem {
    fn normal_use(&self, _item: &Item, _player: &Player) {
        // Drinking is handled by the consumable flow in the server (active hand + consumption tick).
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ItemBehaviour for SplashPotionItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        let stack = player.inventory().get_stack_in_hand(hand);
        self.use_observed_stack(&stack, player, yaw, pitch, hand);
    }

    fn use_observed_stack(
        &self,
        stack: &ItemStack,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) -> bool {
        // ThrowablePotionItem.use consumes getItemInHand(hand); claim before spawning here.
        let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack) else {
            return false;
        };
        let position = player.position();
        let world = player.world();
        world.play_sound(
            Sound::EntityWitchThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
        );
        let entity = Entity::new(world.clone(), position, &EntityType::SPLASH_POTION);
        let splash = SplashPotionEntity::new_shot(entity, player.get_entity());

        splash.set_item_stack(stack.clone());
        let (yaw, pitch) = player.rotation();
        splash.thrown.set_velocity_from(pitch, yaw, 0.0, POWER, 1.0);
        if !crate::entity::projectile::spawn_projectile(
            &world,
            Arc::new(splash),
            Some(player.get_entity()),
        ) {
            crate::item::item_utils::give_or_drop(player, receipt);
            return false;
        }
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ItemBehaviour for LingeringPotionItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        let stack = player.inventory().get_stack_in_hand(hand);
        self.use_observed_stack(&stack, player, yaw, pitch, hand);
    }

    fn use_observed_stack(
        &self,
        stack: &ItemStack,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) -> bool {
        // ThrowablePotionItem.use consumes getItemInHand(hand); claim before spawning here.
        let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack) else {
            return false;
        };
        let position = player.position();
        let world = player.world();
        world.play_sound(
            Sound::EntityWitchThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
        );
        let entity = Entity::new(world.clone(), position, &EntityType::LINGERING_POTION);
        let ling = LingeringPotionEntity::new_shot(entity, player.get_entity());

        ling.set_item_stack(stack.clone());
        let (yaw, pitch) = player.rotation();
        ling.thrown.set_velocity_from(pitch, yaw, 0.0, POWER, 1.0);
        if !crate::entity::projectile::spawn_projectile(
            &world,
            Arc::new(ling),
            Some(player.get_entity()),
        ) {
            crate::item::item_utils::give_or_drop(player, receipt);
            return false;
        }
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
