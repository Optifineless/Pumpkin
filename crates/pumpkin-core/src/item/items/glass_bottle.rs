use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::World;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_util::math::boundingbox::BoundingBox;
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

#[cfg(test)]
#[path = "glass_bottle_tests.rs"]
mod tests;

impl GlassBottleItem {
    // VoxelShape.clip tests occupancy just inside the ray before AABB.clip's entering planes.
    fn clip_shape(
        position: &BlockPos,
        start: Vector3<f64>,
        end: Vector3<f64>,
        shapes: impl Iterator<Item = BoundingBox>,
    ) -> Option<f64> {
        let movement = end - start;
        if movement.length_squared() < 1.0e-7 {
            return None;
        }
        let from = start - position.0.to_f64();
        let test = from + movement * 0.001;
        let mut nearest = None;
        for shape in shapes {
            if test.x >= shape.min.x
                && test.x < shape.max.x
                && test.y >= shape.min.y
                && test.y < shape.max.y
                && test.z >= shape.min.z
                && test.z < shape.max.z
            {
                return Some(0.001);
            }
            if let Some((distance, _)) =
                crate::entity::projectile::clip::clip_box(from, movement, shape)
                && nearest.is_none_or(|best| distance < best)
            {
                nearest = Some(distance);
            }
        }
        nearest
    }

    // BlockGetter.clip with ClipContext.Block.OUTLINE and Fluid.SOURCE_ONLY.
    fn hit_position(world: &World, start: Vector3<f64>, end: Vector3<f64>) -> Option<BlockPos> {
        World::traverse_blocks(start, end, |position, _| {
            let block_hit = Self::clip_shape(
                position,
                start,
                end,
                world
                    .get_block_state(position)
                    .get_block_outline_shapes_at(position),
            );
            let (fluid, state) = world.get_fluid_and_fluid_state(position);
            let fluid_hit = if state.is_source {
                Self::clip_shape(
                    position,
                    start,
                    end,
                    std::iter::once(BoundingBox::new(
                        Vector3::new(0.0, 0.0, 0.0),
                        Vector3::new(
                            1.0,
                            f64::from(world.get_fluid_height(position, fluid, &state)),
                            1.0,
                        ),
                    )),
                )
            } else {
                None
            };
            let hit = if block_hit.unwrap_or(f64::MAX) <= fluid_hit.unwrap_or(f64::MAX) {
                block_hit
            } else {
                fluid_hit
            };
            hit.map(|_| *position)
        })
    }
}

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
        let end = start.add(
            &(Vector3::rotation_vector(f64::from(pitch), f64::from(yaw))
                * player.block_interaction_range()),
        );
        if let Some(position) = Self::hit_position(&world, start, end)
            // BottleItem.use tests the fluid at the hit cell, even when its outline won.
            && world.get_fluid(&position).matches_type(&Fluid::WATER)
        {
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
