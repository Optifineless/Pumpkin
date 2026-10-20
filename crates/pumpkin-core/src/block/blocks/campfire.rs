use pumpkin_data::{
    Block, BlockDirection, BlockState, BlockStateId, Enchantment,
    block_properties::CampfireLikeProperties,
    damage::DamageType,
    data_component_impl::EquipmentSlot,
    effect::StatusEffect,
    fluid::Fluid,
    game_event::GameEvent,
    recipes::{CookingRecipeKind, get_cooking_recipe_with_ingredient},
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
    world::WorldEvent,
};
use pumpkin_inventory::Inventory;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::GameMode;
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

use crate::block::entities::campfire::CampfireBlockEntity;
use crate::{
    block::{
        BlockBehaviour, BlockIsReplacing, GetStateForNeighborUpdateArgs, OnEntityCollisionArgs,
        OnPlaceArgs, PathComputationType, PlacedArgs, UseWithItemArgs, registry::BlockActionResult,
    },
    entity::EntityBase,
};
use std::sync::Arc;

#[pumpkin_block_from_tag("minecraft:campfires")]
pub struct CampfireBlock;

impl BlockBehaviour for CampfireBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = CampfireBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
    }

    fn use_with_item(&self, mut args: UseWithItemArgs<'_>) -> BlockActionResult {
        if let Some(recipe) = get_cooking_recipe_with_ingredient(
            args.item_stack.item,
            CookingRecipeKind::CampfireCooking,
        ) && let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(campfire) = block_entity.as_any().downcast_ref::<CampfireBlockEntity>()
        {
            if let Some(slot) = (0..CampfireBlockEntity::SLOT_COUNT)
                .find(|&slot| campfire.get_stack(slot).is_empty())
            {
                if !campfire.place_food(
                    args.item_stack,
                    args.player,
                    args.world,
                    slot,
                    recipe.cookingtime,
                ) {
                    return BlockActionResult::Consume;
                }
                args.player.increment_stat(
                    pumpkin_data::statistic::StatisticCategory::Custom,
                    pumpkin_data::statistic::CustomStatistic::InteractWithCampfire as i32,
                    1,
                );
                args.world.update_block_entity(&block_entity);
                return BlockActionResult::Success;
            }
            // CampfireBlock.useItemOn consumes food even when all slots are full.
            return BlockActionResult::Consume;
        }

        douse_campfire(&mut args)
    }

    fn on_entity_collision(&self, args: OnEntityCollisionArgs<'_>) {
        if CampfireLikeProperties::from_state_id(args.state.id).lit
            && let Some(living_entity) = args.entity.get_living_entity()
        {
            let has_frost_walker_enchantment = {
                let equipment = living_entity
                    .entity_equipment
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                equipment
                    .equipment
                    .get(&EquipmentSlot::FEET)
                    .is_some_and(|boots| {
                        boots.get_enchantment_level(&Enchantment::FROST_WALKER) != 0
                    })
            };
            let has_fire_res = living_entity
                .get_effect(&StatusEffect::FIRE_RESISTANCE)
                .is_some();
            if has_frost_walker_enchantment || has_fire_res {
                // Campfire damage is prevented by Frost Walker boots or fire resistance.
                return;
            }
            let damage_amount = if args.block == &Block::SOUL_CAMPFIRE {
                2.0
            } else {
                1.0
            };
            args.entity
                .damage(args.entity, damage_amount, DamageType::CAMPFIRE);
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let is_replacing_water = matches!(args.replacing, BlockIsReplacing::Water(_));
        let mut props = CampfireLikeProperties::from_state_id(args.block.default_state.id);
        props.waterlogged = is_replacing_water;
        props.signal_fire = is_signal_fire_base_block(args.world.get_block(&args.position.down()));
        props.lit = !is_replacing_water;
        props.facing = args.player.get_entity().get_horizontal_facing();
        props.to_state_id(args.block)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        let mut props = CampfireLikeProperties::from_state_id(args.state_id);
        if props.waterlogged {
            props.lit = false;
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as u8,
                TickPriority::Normal,
            );
        }

        if args.direction == BlockDirection::Down {
            props.signal_fire =
                is_signal_fire_base_block(args.world.get_block(args.neighbor_position));
        }

        props.to_state_id(args.block)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }

    // TODO: onProjectileHit
}

fn douse_campfire(args: &mut UseWithItemArgs<'_>) -> BlockActionResult {
    // CampfireBlock.useItemOn runs before BlockTransformer's shield-intent guard.
    if !args
        .item_stack
        .item
        .has_tag(&tag::Item::MINECRAFT_DOUSES_CAMPFIRES)
    {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    let observed = args.world.get_block_state_id(args.position);
    if Block::from_state_id(observed) != args.block {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    let mut properties = CampfireLikeProperties::from_state_id(observed);
    if !properties.lit {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    properties.lit = false;
    #[cfg(test)]
    before_douse_write(args.world, *args.position);
    if args.world.get_block_state_id(args.position) != observed {
        return BlockActionResult::Consume;
    }
    args.world
        .sync_world_event(WorldEvent::SoundExtinguishFire, *args.position, 0);
    if args.world.get_block_state_id(args.position) != observed {
        return BlockActionResult::Consume;
    }
    args.world.play_sound_fine(
        Sound::BlockFireExtinguish,
        SoundCategory::Ambient,
        &args.position.to_f64(),
        0.5,
        2.0,
    );
    if args.world.get_block_state_id(args.position) != observed {
        return BlockActionResult::Consume;
    }
    args.world.emit_game_event(
        GameEvent::BlockChange.name(),
        args.position.to_centered_f64(),
    );
    // CampfireBlock.douse publishes before setting LIT; callbacks can replace the source.
    if args
        .world
        .set_block_state_if(
            args.position,
            properties.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
            |current| current == observed,
        )
        .is_none()
    {
        return BlockActionResult::Consume;
    }
    if args.player.gamemode.load() != GameMode::Creative {
        let _ = args.item_stack.damage_item(1);
    }
    BlockActionResult::Success
}

fn is_signal_fire_base_block(block: &Block) -> bool {
    block == &Block::HAY_BLOCK
}

#[cfg(test)]
fn before_douse_write(
    world: &crate::world::World,
    position: pumpkin_util::math::position::BlockPos,
) {
    let hook = world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap()
        .clone();
    if let Some(hook) = hook {
        hook(position);
    }
}

#[cfg(test)]
#[path = "b1020d_g10_campfire_tests.rs"]
mod b1020d_g10_tests;
