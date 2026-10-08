use rand::{RngExt, rng};

use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::ender_pearl::EnderPearlEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
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
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
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
        world.spawn_entity(Arc::new(pearl));

        let mut stack = player.inventory.get_stack_in_hand(hand);
        stack.decrement_unless_creative(player.gamemode.load(), 1);
        player.inventory.set_stack_in_hand(hand, stack);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
