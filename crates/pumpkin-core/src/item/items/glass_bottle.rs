use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::world::World;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, fluid::Fluid};
use pumpkin_util::Hand;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

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
    fn clip_box(from: Vector3<f64>, movement: Vector3<f64>, bounds: BoundingBox) -> Option<f64> {
        const EPSILON: f64 = 1.0e-7;
        let origin = [from.x, from.y, from.z];
        let delta = [movement.x, movement.y, movement.z];
        let min = [bounds.min.x, bounds.min.y, bounds.min.z];
        let max = [bounds.max.x, bounds.max.y, bounds.max.z];
        (0..3)
            .filter_map(|axis| {
                let plane = if delta[axis] > EPSILON {
                    min[axis]
                } else if delta[axis] < -EPSILON {
                    max[axis]
                } else {
                    return None;
                };
                let t = (plane - origin[axis]) / delta[axis];
                (t > 0.0
                    && t < 1.0
                    && (0..3).filter(|&other| other != axis).all(|other| {
                        let coordinate = origin[other] + t * delta[other];
                        coordinate > min[other] - EPSILON && coordinate < max[other] + EPSILON
                    }))
                .then_some(t)
            })
            .min_by(f64::total_cmp)
    }

    // VoxelShape.clip first checks whether the ray starts just inside the shape.
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
            if let Some(distance) = Self::clip_box(from, movement, shape)
                && nearest.is_none_or(|best| distance < best)
            {
                nearest = Some(distance);
            }
        }
        nearest
    }

    /// `BottleItem`'s `ClipContext` uses outline shapes and source fluids, with the main-hand context.
    fn hit_position(
        world: &World,
        start: Vector3<f64>,
        end: Vector3<f64>,
        main_hand: &ItemStack,
    ) -> Option<BlockPos> {
        World::traverse_blocks(start, end, |position, _| {
            let (block, block_state) = world.get_block_and_state(position);
            let held_outline = !main_hand.is_empty()
                && ((block == &Block::SCAFFOLDING && main_hand.item == &Item::SCAFFOLDING)
                    || (block == &Block::LIGHT && main_hand.item == &Item::LIGHT));
            let block_hit = if held_outline {
                Self::clip_shape(
                    position,
                    start,
                    end,
                    std::iter::once(BoundingBox::full_block()),
                )
            } else {
                Self::clip_shape(
                    position,
                    start,
                    end,
                    block_state
                        .get_block_outline_shapes_at(position)
                        .take(block_state.outline_shapes.len()),
                )
            };

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

    /// Returns the target selected by `BottleItem`'s block-outline/source-fluid ray.
    pub(crate) fn use_hit(
        world: &World,
        player: &Player,
        main_hand: &ItemStack,
        yaw: f32,
        pitch: f32,
    ) -> Option<BlockPos> {
        let start = player.eye_position();
        let end = start.add(
            &(Vector3::rotation_vector(f64::from(pitch), f64::from(yaw))
                * player.block_interaction_range()),
        );
        Self::hit_position(world, start, end, main_hand)
    }

    fn may_interact(world: &World, player: &Player, position: &BlockPos) -> bool {
        let within_border = {
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            border.contains_block(position.0.x, position.0.z)
        };
        within_border && !world.is_in_spawn_protection(player, position)
    }

    fn hand_matches(player: &Player, hand: Hand, expected: &ItemStack) -> bool {
        let current = player.inventory().get_stack_in_hand(hand);
        current.uid == expected.uid && current.are_equal(expected)
    }

    /// Uses the hit admitted by the Java pre-use event and the stack observed before that event.
    pub(crate) fn use_packet_snapshot(
        player: &Player,
        hand: Hand,
        expected: &ItemStack,
        hit: Option<BlockPos>,
    ) {
        if expected.is_empty()
            || expected.item != &Item::GLASS_BOTTLE
            || !Self::hand_matches(player, hand, expected)
        {
            return;
        }

        // Dragon-breath collection precedes the water hit, as in BottleItem.use.
        if crate::entity::area_effect_cloud::bottle::try_bottle(player, hand) {
            return;
        }

        let Some(position) = hit else {
            return;
        };
        let world = player.world();
        if !Self::may_interact(&world, player, &position) {
            return;
        }

        let (block, _) = world.get_block_and_state(&position);
        let (fluid, fluid_state) = world.get_fluid_and_fluid_state(&position);
        if block != &Block::WATER
            || !fluid.matches_type(&Fluid::WATER)
            || !fluid_state.is_source
            || !Self::hand_matches(player, hand, expected)
        {
            return;
        }

        world.play_sound_expect(
            player,
            Sound::ItemBottleFill,
            SoundCategory::Neutral,
            &player.position(),
        );
        world.emit_game_event(
            pumpkin_data::game_event::GameEvent::FluidPickup.name(),
            position.to_centered_f64(),
        );
        if !Self::hand_matches(player, hand, expected) {
            return;
        }

        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::GLASS_BOTTLE.id),
            1,
        );
        if !Self::hand_matches(player, hand, expected) {
            return;
        }

        let mut input = expected.clone();
        crate::item::item_utils::create_filled_result(&mut input, player, water_bottle(), true);
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
        let world = player.world();
        let expected = player.inventory().get_stack_in_hand(hand);
        let main_hand = player.inventory().get_stack_in_hand(Hand::Right);
        let hit = Self::use_hit(&world, player, &main_hand, yaw, pitch);
        Self::use_packet_snapshot(player, hand, &expected, hit);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
