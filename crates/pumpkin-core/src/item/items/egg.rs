use std::sync::Arc;

use crate::entity::Entity;
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::entity::projectile::egg::EggEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;

pub struct EggItem;

impl ItemMetadata for EggItem {
    fn ids() -> Box<[u16]> {
        [Item::EGG.id, Item::BLUE_EGG.id, Item::BROWN_EGG.id].into()
    }
}

const POWER: f32 = 1.5;

impl ItemBehaviour for EggItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // EggItem.use reads and consumes player.getItemInHand(hand).
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
        // EggItem.use: claim the hand snapshot before projectile effects in Pumpkin's concurrent tick.
        let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack) else {
            return false;
        };
        let position = player.position();
        let world = player.world();
        world.play_sound(
            Sound::EntityEggThrow,
            pumpkin_data::sound::SoundCategory::Players,
            &position,
        );

        // Capture the held item stack and pass it to the thrown egg entity
        let item_stack = stack.clone();

        let entity = Entity::new(world.clone(), position, &EntityType::EGG);
        let egg = EggEntity::new_shot(entity, player.get_entity());

        // Propagate the item stack so clients show correct variant
        egg.set_item_stack(item_stack);

        let (yaw, pitch) = player.rotation();
        egg.thrown.set_velocity_from(pitch, yaw, 0.0, POWER, 1.0);
        if !world.spawn_entity(Arc::new(egg)) {
            crate::item::item_utils::give_or_drop(player, receipt);
            return false;
        }

        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
