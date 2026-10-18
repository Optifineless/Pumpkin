use rand::{RngExt, rng};

use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::ender_pearl::EnderPearlEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

pub struct EnderPearlItem;

impl ItemMetadata for EnderPearlItem {
    fn ids() -> Box<[u16]> {
        [Item::ENDER_PEARL.id].into()
    }
}

const ROLL: f32 = 0.0;
const POWER: f32 = 1.5;
const DIVERGENCE: f32 = 1.0;
const THROW_SOUND_VOLUME: f32 = 0.5;

impl ItemBehaviour for EnderPearlItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // EnderpearlItem.use reads and consumes player.getItemInHand(hand).
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
        // EnderpearlItem.use: claim the hand snapshot before projectile effects in Pumpkin's concurrent tick.
        let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack) else {
            return false;
        };
        let position = player.position();
        let world = player.world();
        world.play_sound_fine(
            Sound::EntityEnderPearlThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
            THROW_SOUND_VOLUME,
            0.4 / (rng().random::<f32>() * 0.4 + 0.8),
        );

        let entity = Entity::new(world.clone(), position, &EntityType::ENDER_PEARL);
        let pearl = EnderPearlEntity::new_shot(entity, player.get_entity());
        let (yaw, pitch) = player.rotation();
        pearl
            .thrown
            .set_velocity_from(pitch, yaw, ROLL, POWER, DIVERGENCE);
        if !crate::entity::projectile::spawn_projectile(
            &world,
            Arc::new(pearl),
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
