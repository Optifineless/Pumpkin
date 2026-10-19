use std::sync::Arc;

use pumpkin_data::block_properties::DecoratedPotLikeProperties;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_data::{BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::decorated_pot::DecoratedPotBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, BrokenArgs, GetComparatorOutputArgs, NormalUseArgs, OnPlaceArgs,
    OnProjectileHitArgs, PathComputationType, PlacedArgs, PlayerPlacedArgs, PlayerWillDestroyArgs,
    UseWithItemArgs,
};

#[pumpkin_block("minecraft:decorated_pot")]
pub struct DecoratedPotBlock;

#[cfg(test)]
#[path = "decorated_pot_tests.rs"]
mod tests;

impl BlockBehaviour for DecoratedPotBlock {
    fn on_projectile_hit(&self, args: OnProjectileHitArgs<'_>) {
        // DecoratedPotBlock.onProjectileHit: crack only an admitted impact, then use normal loot/removal.
        args.world
            .break_decorated_pot(args.position, args.state.id, args.projectile);
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = DecoratedPotLikeProperties::from_state_id(args.block.default_state.id);
        props.facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = DecoratedPotBlockEntity::new(*args.position);
        args.world.add_block_entity(Arc::new(entity));
    }

    fn player_placed(&self, args: PlayerPlacedArgs<'_>) {
        // BlockItem.updateBlockEntityComponents: publish the claimed faces after application.
        if args.world.get_block(args.position) == args.block
            && let Some(entity) = args.world.get_block_entity(args.position)
            && entity.as_any().is::<DecoratedPotBlockEntity>()
        {
            args.world.update_block_entity(&entity);
        }
    }

    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        if args.item_stack.item_count == 0 {
            return self.normal_use(NormalUseArgs {
                server: args.server,
                world: args.world,
                block: args.block,
                position: args.position,
                player: args.player,
                hit: args.hit,
            });
        }

        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(pot_entity) = block_entity
                .as_any()
                .downcast_ref::<DecoratedPotBlockEntity>()
        {
            if crate::item::item_utils::insert_held_item(
                args.item_stack,
                args.player,
                pot_entity,
                0,
            ) {
                args.world.play_sound(
                    Sound::BlockDecoratedPotInsert,
                    SoundCategory::Blocks,
                    &args.position.to_f64(),
                );
                // Vanilla `BlockEntity.setChanged` notifies comparators.
                args.world
                    .update_neighbour_for_output_signal(args.position, args.block);
            } else {
                args.world.play_sound(
                    Sound::BlockDecoratedPotInsertFail,
                    SoundCategory::Blocks,
                    &args.position.to_f64(),
                );
            }
            return BlockActionResult::Success;
        }

        BlockActionResult::Pass
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        args.world.play_sound(
            Sound::BlockDecoratedPotInsertFail,
            SoundCategory::Blocks,
            &args.position.to_f64(),
        );
        BlockActionResult::Success
    }

    fn broken(&self, args: BrokenArgs<'_>) {
        args.world.play_sound(
            Sound::BlockDecoratedPotShatter,
            SoundCategory::Blocks,
            &args.position.to_f64(),
        );
    }

    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        let held_item = args.player.inventory().held_item();
        if held_item.is_empty()
            || !held_item
                .item
                .has_tag(&tag::Item::MINECRAFT_BREAKS_DECORATED_POTS)
        {
            return;
        }

        let mut prevents_shattering = false;
        crate::enchantment::EnchantmentHelper::run_iteration_on_item(
            &held_item,
            |enchantment, _| {
                prevents_shattering |= enchantment
                    .has_tag(&tag::Enchantment::MINECRAFT_PREVENTS_DECORATED_POT_SHATTERING);
            },
        );
        if prevents_shattering {
            return;
        }

        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(pot_entity) = block_entity
                .as_any()
                .downcast_ref::<DecoratedPotBlockEntity>()
            && pot_entity.has_empty_decorations()
        {
            return;
        }

        let mut properties = DecoratedPotLikeProperties::from_state_id(args.state.id);
        properties.cracked = true;
        // Mirrors vanilla's setBlock(..., 260): UPDATE_KNOWN_SHAPE | NOTIFY_LISTENERS.
        args.world.set_block_state(
            args.position,
            properties.to_state_id(args.block),
            BlockFlags::UPDATE_KNOWN_SHAPE | BlockFlags::NOTIFY_LISTENERS,
        );
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(pot_entity) = block_entity
                .as_any()
                .downcast_ref::<DecoratedPotBlockEntity>()
        {
            Some(pot_entity.get_comparator_output())
        } else {
            Some(0)
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
