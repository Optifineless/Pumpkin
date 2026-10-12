use crate::block::registry::BlockActionResult;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::firework_rocket::FireworkRocketEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct FireworkRocketItem;

impl ItemMetadata for FireworkRocketItem {
    fn ids() -> Box<[u16]> {
        [Item::FIREWORK_ROCKET.id].into()
    }
}

impl ItemBehaviour for FireworkRocketItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        cursor_pos: Vector3<f32>,
        _block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        let world = player.world();
        let entity = Entity::new(
            world.clone(),
            Vector3::new(
                f64::from(location.0.x) + f64::from(cursor_pos.x),
                f64::from(location.0.y) + f64::from(cursor_pos.y),
                f64::from(location.0.z) + f64::from(cursor_pos.z),
            ),
            &EntityType::FIREWORK_ROCKET,
        );
        let entity =
            FireworkRocketEntity::with_item(entity, item.clone(), Some(player.get_entity()), false);
        world.spawn_entity(Arc::new(entity));
        item.decrement_unless_creative(player.gamemode.load(), 1);
        BlockActionResult::Success
    }

    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // FireworkRocketItem.use reads and consumes player.getItemInHand(hand).
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
        observed: &pumpkin_data::item_stack::ItemStack,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) -> bool {
        if player.get_entity().is_fall_flying() {
            if observed.is_empty() || observed.item.id != Item::FIREWORK_ROCKET.id {
                return false;
            }
            let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, observed)
            else {
                return false;
            };

            let world = player.world();
            let entity = Entity::new(
                world.clone(),
                player.get_entity().pos.load(),
                &EntityType::FIREWORK_ROCKET,
            );
            // FireworkRocketItem.use builds the rocket from the hand's stack, attached to the player.
            let rocket = FireworkRocketEntity::with_item(
                entity,
                observed.clone(),
                Some(player.get_entity()),
                true,
            );
            if !world.spawn_entity(Arc::new(rocket)) {
                crate::item::item_utils::give_or_drop(player, receipt);
                return false;
            }
            return true;
        }
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
