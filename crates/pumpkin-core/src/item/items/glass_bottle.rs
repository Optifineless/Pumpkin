use std::any::Any;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// A potion holding water; the contents component is what names it and makes it
/// brewable.
#[must_use]
pub fn water_bottle() -> ItemStack {
    ItemStack::new_with_component(
        1,
        &Item::POTION,
        vec![(
            DataComponent::PotionContents,
            Some(Box::new(PotionContentsImpl {
                potion_id: Some(i32::from(Potion::WATER.id)),
                custom_color: None,
                custom_effects: Vec::new(),
                custom_name: None,
            }) as Box<_>),
        )],
    )
}

pub struct GlassBottleItem;

impl ItemMetadata for GlassBottleItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::GLASS_BOTTLE.id])
    }
}

impl ItemBehaviour for GlassBottleItem {
    // BottleItem inherits Item.useOn's Pass; CauldronInteractions handles cauldrons.
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_hand(item, player, yaw, pitch, pumpkin_util::Hand::Right);
    }

    // BottleItem.use uses getItemInHand(hand), including when both hands hold bottles.
    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: pumpkin_util::Hand,
    ) {
        // BottleItem.use bottles dragon breath from a nearby cloud before the water raycast.
        if crate::entity::area_effect_cloud::bottle::try_bottle(player, hand) {
            return;
        }
        let world = player.world();
        let start = player.eye_position();
        let end = start.add(&(Vector3::rotation_vector(f64::from(pitch), f64::from(yaw)) * 4.5));
        let checker = |pos: &BlockPos, world: &Arc<World>| {
            let state = world.get_block_state_id(pos);
            let block = Block::from_state_id(state);
            block.id == Block::WATER.id || block.is_waterlogged(state)
        };
        if let Some((position, _)) = world.raycast(start, end, checker) {
            let mut stack = player.inventory().get_stack_in_hand(hand);
            if stack.is_empty() || stack.item != &Item::GLASS_BOTTLE {
                return;
            }
            world.play_sound(
                Sound::ItemBottleFill,
                SoundCategory::Players,
                &position.to_f64(),
            );
            crate::item::item_utils::create_filled_result(&mut stack, player, water_bottle(), true);
            player.inventory().set_stack_in_hand(hand, stack);
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Used,
                i32::from(Item::GLASS_BOTTLE.id),
                1,
            );
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
