use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::snowball::SnowballEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

pub struct SnowBallItem;

impl ItemMetadata for SnowBallItem {
    fn ids() -> Box<[u16]> {
        [Item::SNOWBALL.id].into()
    }
}

const POWER: f32 = 1.5;

impl ItemBehaviour for SnowBallItem {
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
        // SnowballItem.use pays the observed hand with the first admitted insertion.
        let Some(mut source) =
            crate::item::projectile_stock::ProjectileSource::plan_hand(player, hand, stack)
        else {
            return false;
        };
        let position = player.position();
        let world = player.world();
        let entity = Entity::new(world.clone(), position, &EntityType::SNOWBALL);
        let snowball = SnowballEntity::new_shot(entity, player.get_entity());
        let (yaw, pitch) = player.rotation();
        snowball
            .thrown
            .set_velocity_from(pitch, yaw, 0.0, POWER, 1.0);
        if !source.launch(player, Arc::new(snowball)) {
            return false;
        }
        world.play_sound(
            Sound::EntitySnowballThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
        );

        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
