use std::any::Any;

use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::net::java::play::hand_use_result::hand_slot;
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::PotionContentsImpl;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::statistic::StatisticCategory;
use pumpkin_inventory::{
    Inventory, player::player_inventory::PlayerInventory, screen_handler::InventoryPlayer,
};
use pumpkin_util::Hand;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilledBottleCommit {
    Stored,
    Overflow,
    Rejected,
}

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
    fn hit_position(
        world: &World,
        start: Vector3<f64>,
        end: Vector3<f64>,
        main_hand: &ItemStack,
    ) -> Option<BlockPos> {
        World::traverse_blocks(start, end, |position, _| {
            let (block, block_state) = world.get_block_and_state(position);
            // EntityCollisionContext uses the main hand, including for an offhand bottle.
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
                    block_state.get_block_outline_shapes_at(position),
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

    // BottleItem.use must pass Level.mayInteract before filling a water bottle.
    fn may_interact(world: &World, player: &Player, position: &BlockPos) -> bool {
        {
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let x = f64::from(position.0.x);
            let z = f64::from(position.0.z);
            let limit = f64::from(border.portal_teleport_boundary);
            // WorldBorder.isWithinBounds(BlockPos) tests the integer block origin.
            if !border.contains(x, z)
                || !(-limit..limit).contains(&x)
                || !(-limit..limit).contains(&z)
            {
                return false;
            }
        }

        // The fork persists the shared respawn dimension as the Overworld.
        if world.dimension != Dimension::OVERWORLD {
            return true;
        }
        let Some(server) = world.server.upgrade() else {
            return true;
        };
        let radius = server.basic_config.spawn_protection;
        if radius == 0 {
            return true;
        }
        {
            let operators = server
                .data
                .operator_config
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // DedicatedServer tests operator membership, regardless of permission level.
            if operators.ops.is_empty() || operators.get_entry(&player.gameprofile.id).is_some() {
                return true;
            }
        }
        let spawn = world.get_spawn_location().0;
        position.0.x.abs_diff(spawn.0.x) > radius || position.0.z.abs_diff(spawn.0.z) > radius
    }

    // ItemUtils.createFilledResult consumes the hand before publishing a dropped result.
    fn commit_hand_item(
        player: &Player,
        source_slot: usize,
        before: &ItemStack,
        after: &ItemStack,
    ) -> bool {
        let inventory = player.inventory();
        let stored = if after.is_empty() {
            ItemStack::EMPTY.clone()
        } else {
            after.clone()
        };
        let committed = if source_slot < PlayerInventory::MAIN_SIZE {
            let mut items = inventory
                .main_inventory
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(current) = items.get_mut(source_slot) else {
                return false;
            };
            if current.uid != before.uid || !current.are_equal(before) {
                false
            } else {
                *current = stored.clone();
                true
            }
        } else if let Some(equipment_slot) = inventory.equipment_slots.get(&source_slot) {
            let mut equipment = inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = equipment.get(equipment_slot);
            if current.uid != before.uid || !current.are_equal(before) {
                false
            } else {
                equipment.put(equipment_slot, stored.clone());
                true
            }
        } else {
            false
        };

        if committed {
            player.sync_hand_slot(source_slot, stored);
        }
        committed
    }

    // ItemUtils.createFilledResult and PlayerInventory.insert_stack_anywhere share this owner-side exchange.
    fn commit_hand_and_insert(
        player: &Player,
        source_slot: usize,
        before: &ItemStack,
        after: &ItemStack,
        output: &ItemStack,
    ) -> FilledBottleCommit {
        fn can_stack(existing: &ItemStack, output: &ItemStack) -> bool {
            !existing.is_empty()
                && existing.are_items_and_components_equal(output)
                && existing.is_stackable()
                && existing.item_count.saturating_add(output.item_count)
                    <= existing.get_max_stack_size()
        }

        let inventory = player.inventory();
        let needs_equipment = source_slot >= PlayerInventory::MAIN_SIZE || output.is_stackable();
        let mut items = inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut equipment = needs_equipment.then(|| {
            inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        let source_matches = if source_slot < PlayerInventory::MAIN_SIZE {
            items
                .get(source_slot)
                .is_some_and(|stack| stack.uid == before.uid && stack.are_equal(before))
        } else if let (Some(slot), Some(equipment)) = (
            inventory.equipment_slots.get(&source_slot),
            equipment.as_ref(),
        ) {
            let stack = equipment.get(slot);
            stack.uid == before.uid && stack.are_equal(before)
        } else {
            false
        };
        if !source_matches {
            return FilledBottleCommit::Rejected;
        }

        let selected = inventory.get_selected_slot() as usize;
        let off_hand_slot = inventory
            .equipment_slots
            .get(&PlayerInventory::OFF_HAND_SLOT);
        let destination = if can_stack(&items[selected], output) {
            Some(selected)
        } else if off_hand_slot.is_some_and(|slot| {
            equipment
                .as_ref()
                .is_some_and(|equipment| can_stack(&equipment.get(slot), output))
        }) {
            Some(PlayerInventory::OFF_HAND_SLOT)
        } else if let Some((slot, _)) = items
            .iter()
            .enumerate()
            .find(|(_, stack)| can_stack(stack, output))
        {
            Some(slot)
        } else {
            items.iter().position(ItemStack::is_empty)
        };
        let Some(destination) = destination else {
            return FilledBottleCommit::Overflow;
        };

        let stored = if after.is_empty() {
            ItemStack::EMPTY.clone()
        } else {
            after.clone()
        };
        if source_slot < PlayerInventory::MAIN_SIZE {
            items[source_slot] = stored.clone();
        } else if let (Some(slot), Some(equipment)) = (
            inventory.equipment_slots.get(&source_slot),
            equipment.as_mut(),
        ) {
            equipment.put(slot, stored.clone());
        } else {
            return FilledBottleCommit::Rejected;
        }

        if destination == PlayerInventory::OFF_HAND_SLOT {
            if let (Some(slot), Some(equipment)) = (off_hand_slot, equipment.as_mut()) {
                let mut stack = equipment.get(slot);
                stack.increment(output.item_count);
                equipment.put(slot, stack);
            }
        } else if items[destination].is_empty() {
            items[destination] = output.clone();
        } else {
            items[destination].increment(output.item_count);
        }

        drop(equipment);
        drop(items);
        player.sync_hand_slot(source_slot, stored);
        FilledBottleCommit::Stored
    }

    // BottleItem.turnBottleIntoItem follows ItemUtils.createFilledResult's creative and consume branches.
    fn fill_water_bottle(
        world: &std::sync::Arc<World>,
        player: &Player,
        source_slot: usize,
        before: &ItemStack,
        position: &BlockPos,
    ) {
        let output = water_bottle();
        if player.has_infinite_materials() {
            let mut stack = before.clone();
            crate::item::item_utils::create_filled_result(&mut stack, player, output, true);
            return;
        }

        let mut after = before.clone();
        after.decrement_unless_creative(player.gamemode.load(), 1);
        if after.is_empty() {
            let _ = Self::commit_hand_item(player, source_slot, before, &output);
            return;
        }

        match Self::commit_hand_and_insert(player, source_slot, before, &after, &output) {
            FilledBottleCommit::Stored | FilledBottleCommit::Rejected => return,
            FilledBottleCommit::Overflow => {}
        }

        world.drop_stack_with_commit(position, output, || {
            Self::commit_hand_item(player, source_slot, before, &after)
        });
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
        self.normal_use_with_hand(item, player, yaw, pitch, Hand::Right);
    }

    // BottleItem.use uses getItemInHand(hand), including when both hands hold bottles.
    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: Hand,
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
        let main_hand = player.inventory().get_stack_in_hand(Hand::Right);
        if let Some(position) = Self::hit_position(&world, start, end, &main_hand)
            && Self::may_interact(&world, player, &position)
            // BottleItem.use tests the fluid at the hit cell, even when its outline won.
            && world.get_fluid(&position).matches_type(&Fluid::WATER)
        {
            let source_slot = hand_slot(player, hand);
            let before = player.inventory().get_stack(source_slot);
            if before.is_empty() || before.item != &Item::GLASS_BOTTLE {
                return;
            }
            let hand_unchanged = || {
                let current = player.inventory().get_stack(source_slot);
                current.uid == before.uid && current.are_equal(&before)
            };
            world.play_sound_expect(
                player,
                Sound::ItemBottleFill,
                SoundCategory::Neutral,
                &player.position(),
            );
            world.emit_game_event(GameEvent::FluidPickup.name(), position.to_centered_f64());
            // BottleItem emits the event and awards its stat before exchanging the bottle.
            // Preserve direct writers and retain the original slot when a callback changes selection.
            if !hand_unchanged() {
                return;
            }
            player.increment_stat(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id), 1);
            if !hand_unchanged() {
                return;
            }
            Self::fill_water_bottle(&world, player, source_slot, &before, &position);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
