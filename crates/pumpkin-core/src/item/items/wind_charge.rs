use std::sync::Arc;

use crate::entity::player::Player;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::projectile::ThrownItemEntity;
use crate::entity::projectile::wind_charge::{WIND_CHARGE_GRAVITY, WindChargeEntity};
use crate::item::{ItemBehaviour, ItemMetadata};

pub struct WindChargeItem;

impl ItemMetadata for WindChargeItem {
    fn ids() -> Box<[u16]> {
        [Item::WIND_CHARGE.id].into()
    }
}

const POWER: f32 = 1.5;

impl ItemBehaviour for WindChargeItem {
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
        // WindChargeItem.use consumes the observed hand, before effects in the concurrent tick.
        let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack) else {
            return false;
        };
        let world = player.world();
        let position = player.position();

        world.play_sound(
            Sound::EntityWindChargeThrow,
            pumpkin_data::sound::SoundCategory::Neutral,
            &position,
        );

        let entity = Entity::new(world.clone(), position, &EntityType::WIND_CHARGE);

        let wind_charge = ThrownItemEntity::new(entity, player.get_entity(), WIND_CHARGE_GRAVITY);
        let (yaw, pitch) = player.rotation();
        wind_charge.set_velocity_from(pitch, yaw, 0.0, POWER, 1.0);

        if !world.spawn_entity(Arc::new(WindChargeEntity::new_normal(wind_charge))) {
            crate::item::item_utils::give_or_drop(player, receipt);
            return false;
        }

        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
