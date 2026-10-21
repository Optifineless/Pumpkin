use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockStateId, tag, world::WorldEvent};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

use super::Player;
use crate::{world::World, world::guard_effects::defer};

// AnvilMenu.onTake's access callback (26.3 AnvilMenu.java:102).
const ANVIL_DAMAGE_CHANCE: f32 = 0.12;

impl Player {
    pub(super) fn use_menu_anvil(&self) {
        // AnvilMenu.onTake uses the menu's captured ContainerLevelAccess.
        let Some(pos) = self.open_container_pos.load() else {
            return;
        };
        let world = self.world();
        let infinite_materials = self.has_infinite_materials();
        let effect = move || on_take_anvil(&world, pos, infinite_materials);
        if let Err(effect) = defer(effect) {
            effect();
        }
    }

    pub(super) fn use_menu_grindstone(&self, xp_amount: i32) {
        // GrindstoneMenu's result Slot.onTake uses the menu's captured access.
        let Some(pos) = self.open_container_pos.load() else {
            return;
        };
        let world = self.world();
        let effect = move || {
            if xp_amount > 0 {
                crate::entity::experience_orb::ExperienceOrbEntity::spawn(
                    &world,
                    pos.to_centered_f64(),
                    xp_amount as u32,
                );
            }
            world.sync_world_event(WorldEvent::SoundGrindstoneUsed, pos, 0);
        };
        if let Err(effect) = defer(effect) {
            effect();
        }
    }
}

fn on_take_anvil(world: &Arc<World>, pos: BlockPos, infinite_materials: bool) {
    let state = world.get_block_state(&pos);
    let block = Block::from_state_id(state.id);
    if block.has_tag(&tag::Block::MINECRAFT_ANVIL)
        && !infinite_materials
        && rand::random::<f32>() < ANVIL_DAMAGE_CHANCE
    {
        if let Some(new_state) = crate::block::blocks::anvil::AnvilBlock::damage(state.id) {
            if world.set_block_state_from_observed(
                &pos,
                state.id,
                new_state,
                BlockFlags::NOTIFY_ALL,
            ) {
                world.sync_world_event(WorldEvent::SoundAnvilUsed, pos, 0);
            }
        } else if world.set_block_state_from_observed(
            &pos,
            state.id,
            BlockStateId::AIR,
            BlockFlags::NOTIFY_ALL,
        ) {
            world.sync_world_event(WorldEvent::SoundAnvilBroken, pos, 0);
        }
    } else {
        world.sync_world_event(WorldEvent::SoundAnvilUsed, pos, 0);
    }
}
