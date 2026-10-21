use std::any::Any;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::{BlockFlags, World};
use pumpkin_data::Block;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::item::Item;
use pumpkin_data::sound::Sound;
use pumpkin_util::math::position::BlockPos;

pub struct PlaceOnWaterBlockItem;

impl ItemMetadata for PlaceOnWaterBlockItem {
    fn ids() -> Box<[u16]> {
        [Item::LILY_PAD.id, Item::FROGSPAWN.id].into()
    }
}

impl ItemBehaviour for PlaceOnWaterBlockItem {
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
        stack: &pumpkin_data::item_stack::ItemStack,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: pumpkin_util::Hand,
    ) -> bool {
        let item = stack.item;
        let world = player.world();
        let (start_pos, end_pos) = self.get_start_and_end_pos(player);
        let checker = |pos: &BlockPos, world_inner: &Arc<World>| {
            let state_id = world_inner.get_block_state_id(pos);
            if state_id == Block::AIR.default_state.id {
                return false;
            }
            Fluid::from_state_id(state_id).is_some()
        };

        let Some((hit_pos, _)) = world.raycast(start_pos, end_pos, checker) else {
            return false;
        };

        let hit_state = world.get_block_state_id(&hit_pos);
        if Fluid::from_state_id(hit_state).is_none() {
            return false;
        }
        let above_pos = hit_pos.up();
        let above_state = world.get_block_state(&above_pos);
        if above_state.is_air() {
            let (placed_block, sound) = if item.id == Item::LILY_PAD.id {
                (&Block::LILY_PAD, Sound::BlockLilyPadPlace)
            } else {
                (&Block::FROGSPAWN, Sound::BlockFrogspawnPlace)
            };

            // PlaceOnWaterBlockItem.use / BlockItem.place consume the same used hand.
            let Some(receipt) = crate::item::item_utils::claim_item_in_hand(player, hand, stack)
            else {
                return false;
            };
            if !world.set_block_states_from_observed(&[
                (hit_pos, hit_state, hit_state, BlockFlags::empty()),
                (
                    above_pos,
                    above_state.id,
                    placed_block.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                ),
            ]) {
                crate::item::item_utils::give_or_drop(player, receipt);
                return false;
            }
            world.play_sound(
                sound,
                pumpkin_data::sound::SoundCategory::Blocks,
                &above_pos.to_f64(),
            );

            return true;
        }
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
