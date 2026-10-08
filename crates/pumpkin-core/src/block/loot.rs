use crate::{
    entity::experience_orb::ExperienceOrbEntity,
    world::{World, loot::LootContextParameters},
};
use pumpkin_data::Block;
use pumpkin_util::{
    math::position::BlockPos,
    random::{RandomGenerator, get_seed, xoroshiro128::Xoroshiro},
};
use std::sync::Arc;

/// Returns the block name whose loot table should be used.
/// Some blocks (e.g. wall torches) share the loot table of their
/// non-wall counterpart but do not have their own loot table file.
#[must_use]
pub fn loot_table_name(block: &Block) -> &str {
    match block.name {
        "wall_torch" => "torch",
        "soul_wall_torch" => "soul_torch",
        "copper_wall_torch" => "copper_torch",
        "redstone_wall_torch" => "redstone_torch",
        _ => block.name,
    }
}

pub fn drop_loot(
    world: &Arc<World>,
    block: &Block,
    pos: &BlockPos,
    experience: bool,
    params: &LootContextParameters,
) {
    for stack in block_drops(world, block, pos, params) {
        world.drop_stack(pos, stack);
    }

    if experience {
        drop_experience(world, block, pos, params.tool.as_ref());
    }
}

/// Evaluates complete block loot and its plugin event before callers collect or spawn the drops.
pub fn block_drops(
    world: &Arc<World>,
    block: &Block,
    pos: &BlockPos,
    params: &LootContextParameters,
) -> Vec<pumpkin_data::item_stack::ItemStack> {
    // Block.getDrops / BlockEntity.collectComponents; Blocks.wallVariant assigns the standing block's loot table (upstream #3890).
    let key = format!("minecraft:blocks/{}", loot_table_name(block));
    if let Some(loot_table) = world.get_loot_table(&key) {
        let params = crate::world::loot::build_block_loot_context(world, pos, params);
        let items = crate::world::loot::generate_loot_from_handle(&loot_table, 0, &params);
        if !items.is_empty() {
            let mut event = crate::plugin::block::block_drop_item::BlockDropItemEvent {
                block_pos: *pos,
                world: world.clone(),
                player: None,
                items,
                cancelled: false,
            };
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if !event.cancelled {
                return event.items;
            }
        }
    }

    Vec::new()
}

/// Drops a block's experience without generating its item loot.
// DropExperienceBlock.spawnAfterBreak / Block.popExperience; also used by player-caused explosions.
pub fn drop_experience(
    world: &Arc<World>,
    block: &Block,
    pos: &BlockPos,
    tool: Option<&pumpkin_data::item_stack::ItemStack>,
) {
    let has_silk_touch = tool.is_some_and(|tool| {
        pumpkin_data::Enchantment::from_name("silk_touch")
            .is_some_and(|e| tool.get_enchantment_level(e) > 0)
    });

    if !has_silk_touch && let Some(experience) = &block.experience {
        let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed()));
        let amount = experience.experience.get(&mut random);
        if amount > 0 {
            let mut event = crate::plugin::block::block_exp::BlockExpEvent {
                block_pos: *pos,
                world: world.clone(),
                exp: amount,
            };
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.exp > 0 {
                // Block.popExperience uses Vec3.atCenterOf.
                ExperienceOrbEntity::award(world, pos.to_centered_f64(), event.exp as u32);
            }
        }
    }
}
