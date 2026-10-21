use std::sync::Arc;

use pumpkin_data::{Block, BlockStateId, tag::Taggable};
use pumpkin_util::sync::{SnapshotBoundary as SB, assert_not_in_snapshot};
use pumpkin_util::{math::position::BlockPos, text::TextComponent};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};
use rustc_hash::FxHashSet;

use super::{World, neighbor_updater::UPDATE_LIMIT};
use crate::{entity::player::Player, plugin::api::events::block::block_break::BlockBreakEvent};

fn keeps_block_entity(old: BlockStateId, new: BlockStateId) -> bool {
    old.to_block() == new.to_block()
        || (old
            .to_block()
            .has_tag(&pumpkin_data::tag::Block::MINECRAFT_COPPER_CHESTS)
            && new
                .to_block()
                .has_tag(&pumpkin_data::tag::Block::MINECRAFT_COPPER_CHESTS))
}

struct ObservedBlockCommit {
    position: BlockPos,
    old: BlockStateId,
    state: BlockStateId,
    flags: BlockFlags,
    retired: Option<super::portal::frame_effects::RetiredBlockEntity>,
    effects: super::portal::frame_effects::RetiredBlockEffects,
}

impl ObservedBlockCommit {
    fn publish(self, world: &Arc<World>, update_limit: u32) {
        let Self {
            position,
            old,
            state,
            flags,
            retired,
            effects,
        } = self;
        if let Some(retired) = retired {
            if !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK) {
                retired.on_block_replaced(world, &position);
            }
            world.on_block_state_set_with_block_entity(
                &position,
                old,
                state,
                flags,
                update_limit,
                Some(&effects),
            );
        } else {
            world.on_block_state_set(&position, old, state, flags, update_limit);
        }
    }
}

impl World {
    pub(super) fn commit_block_state_if(
        self: &Arc<Self>,
        position: &BlockPos,
        state: BlockStateId,
        flags: BlockFlags,
        update_limit: u32,
        condition: impl Fn(BlockStateId) -> bool,
    ) -> Option<BlockStateId> {
        if !self.is_in_build_limit(*position) {
            return None;
        }
        #[cfg(test)]
        self.before_state_write_test(*position);
        let _ = self.get_block_entity(position);
        let mutation = self.reserve_block_entity_mutation(*position)?;
        let (old, retired, effects) = {
            let mut loads = self
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let old = self.write_block_state_if(position, state, condition)?;
            let effects =
                super::portal::frame_effects::RetiredBlockEffects::capture(self, position, old);
            let retired = (!keeps_block_entity(old, state))
                .then(|| self.remove_block_entity_under_publication_gate(position, &mut loads));
            (old, retired, effects)
        };
        drop(mutation);
        if let Some(retired) = &retired {
            retired.retire_owner();
        }
        ObservedBlockCommit {
            position: *position,
            old,
            state,
            flags,
            retired,
            effects,
        }
        .publish(self, update_limit);
        Some(old)
    }

    /// Replaces a state only while the operation's original observation still holds.
    pub fn set_block_state_from_observed(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        state: BlockStateId,
        flags: BlockFlags,
    ) -> bool {
        self.set_block_state_if(position, state, flags, |current| current == expected)
            .is_some()
    }

    /// Admits every observed cell before committing any paired or dependent block change.
    /// No lock, guard or mutation permit survives into world callbacks.
    pub fn set_block_states_from_observed(
        self: &Arc<Self>,
        updates: &[(BlockPos, BlockStateId, BlockStateId, BlockFlags)],
    ) -> bool {
        assert_not_in_snapshot(SB::Mutation, "World::set_block_states_from_observed");
        #[cfg(test)]
        for &(position, _, _, _) in updates {
            self.before_state_write_test(position);
        }
        for &(position, _, _, _) in updates {
            let _ = self.get_block_entity(&position);
        }
        let mut positions = FxHashSet::default();
        let mut permits = Vec::new();
        let mut chunks = Vec::new();
        let mut raw = Vec::with_capacity(updates.len());
        for &(position, expected, state, _) in updates {
            if !self.is_in_build_limit(position) || !positions.insert(position) {
                return false;
            }
            let Some(permit) = self.reserve_block_entity_mutation(position) else {
                return false;
            };
            permits.push(permit);
            let coordinate = position.chunk_position();
            let Some(chunk) = self.level.read_chunk_sync(&coordinate, Arc::clone) else {
                return false;
            };
            if !chunks.iter().any(|existing| Arc::ptr_eq(existing, &chunk)) {
                chunks.push(chunk);
            }
            raw.push((position, expected, state));
        }
        let committed = {
            let mut loads = self
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !ChunkData::set_observed_blocks(&chunks, &raw) {
                return false;
            }
            updates
                .iter()
                .map(|&(position, expected, state, flags)| {
                    let effects = super::portal::frame_effects::RetiredBlockEffects::capture(
                        self, &position, expected,
                    );
                    let retired = (!keeps_block_entity(expected, state)).then(|| {
                        self.remove_block_entity_under_publication_gate(&position, &mut loads)
                    });
                    (position, expected, state, flags, retired, effects)
                })
                .collect::<Vec<_>>()
        };
        drop(permits);
        for (_, _, _, _, retired, _) in &committed {
            if let Some(retired) = retired {
                retired.retire_owner();
            }
        }
        for (position, expected, state, flags, retired, effects) in committed {
            if expected != state || flags.contains(BlockFlags::FORCE_STATE) {
                ObservedBlockCommit {
                    position,
                    old: expected,
                    state,
                    flags,
                    retired,
                    effects,
                }
                .publish(self, UPDATE_LIMIT);
            }
        }
        true
    }

    /// Breaks only the state on which the caller based its decision.
    pub fn break_block_from_observed(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        cause: Option<&Arc<Player>>,
        flags: BlockFlags,
    ) -> Option<BlockStateId> {
        self.break_block_with_limit_from_observed(position, cause, flags, UPDATE_LIMIT, expected)
    }

    fn observed_break_flags(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        expected: BlockStateId,
        flags: BlockFlags,
    ) -> Option<BlockFlags> {
        let broken_block = expected.to_block();
        let mut flags = flags;
        if flags.contains(BlockFlags::SKIP_DROPS)
            && cause.is_some_and(|p| p.gamemode.load() == pumpkin_util::GameMode::Creative)
            && self
                .get_block_entity(position)
                .is_some_and(|entity| entity.drops_for_creative_player())
        {
            flags.remove(BlockFlags::SKIP_DROPS);
        }

        let mut event = BlockBreakEvent::new(
            cause.cloned(),
            broken_block,
            *position,
            0,
            !flags.contains(BlockFlags::SKIP_DROPS),
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled || self.get_block_state_id(position) != expected {
            return None;
        }

        if event.drop {
            flags.remove(BlockFlags::SKIP_DROPS);
        } else {
            flags.insert(BlockFlags::SKIP_DROPS);
        }

        Some(flags)
    }

    pub(super) fn break_block_with_limit_from_observed(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        flags: BlockFlags,
        update_limit: u32,
        expected: BlockStateId,
    ) -> Option<BlockStateId> {
        let replacement = crate::block::blocks::growing_plant::fluid_state_after_break(expected);
        self.replace_block_with_limit_from_observed(
            position,
            cause,
            flags,
            update_limit,
            expected,
            replacement,
        )
    }

    pub(crate) fn replace_block_from_observed(
        self: &Arc<Self>,
        position: &BlockPos,
        expected: BlockStateId,
        replacement: BlockStateId,
        cause: Option<&Arc<Player>>,
        flags: BlockFlags,
    ) -> Option<BlockStateId> {
        self.replace_block_with_limit_from_observed(
            position,
            cause,
            flags,
            UPDATE_LIMIT,
            expected,
            replacement,
        )
    }

    fn replace_block_with_limit_from_observed(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        flags: BlockFlags,
        update_limit: u32,
        expected: BlockStateId,
        new_state_id: BlockStateId,
    ) -> Option<BlockStateId> {
        if let Some(player) = cause
            && self.is_in_spawn_protection(player, position)
        {
            player.send_system_message(&TextComponent::translate_cross(
                pumpkin_data::translation::java::BUILD_SPAWN_PROTECTION,
                pumpkin_data::translation::java::BUILD_SPAWN_PROTECTION,
                [TextComponent::text(player.gameprofile.name.clone())],
            ));
            return None;
        }

        let broken_block_state = expected.to_state();
        let broken_block = expected.to_block();
        if self.get_block_state_id(position) != expected || broken_block_state.is_air() {
            return None;
        }

        if broken_block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEDS)
            || broken_block == &Block::STRAW_BED
        {
            return self.break_bed_block(
                position,
                cause,
                cause,
                flags,
                update_limit,
                broken_block_state.id,
            );
        }

        let (removal_state, flags) =
            self.prepare_observed_removal(position, cause, expected, flags)?;

        let prepared = (!flags.contains(BlockFlags::SKIP_DROPS))
            .then(|| self.prepare_observed_block_loot(position, removal_state, cause));
        let experience = prepared.as_ref().map_or(0, |drops| {
            crate::block::prepare_block_experience(self, broken_block, position, drops.tool())
        });

        #[cfg(test)]
        self.before_state_write_test(*position);
        let _ = self.get_block_entity(position);
        let retired_effects = super::portal::frame_effects::RetiredBlockEffects::capture(
            self,
            position,
            removal_state,
        );
        let mutation = self.reserve_block_entity_mutation(*position)?;
        let retired = self.remove_observed_block(position, removal_state, new_state_id)?;
        drop(mutation);
        let broken_state_id = removal_state;
        let entity = retired.live.clone().or_else(|| {
            retired
                .pending
                .as_ref()
                .and_then(crate::block::entities::block_entity_from_nbt)
        });
        if let Some(entity) = &entity {
            #[cfg(test)]
            crate::inventory_transaction_followup_tests::pause_retirement();
            super::container_lifecycle::retire_block_entity(entity);
        }
        if let Some(prepared) = prepared {
            for stack in prepared.finish(self, position, entity.as_deref()) {
                self.drop_stack(position, stack);
            }
            if experience > 0 {
                crate::entity::experience_orb::ExperienceOrbEntity::award(
                    self,
                    position.to_centered_f64(),
                    experience,
                );
            }
        }
        if !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK) {
            retired.on_block_replaced(self, position);
        }
        self.on_block_state_set_with_block_entity(
            position,
            broken_state_id,
            new_state_id,
            flags - BlockFlags::SKIP_DROPS,
            update_limit,
            Some(&retired_effects),
        );
        let broken_block = Block::from_state_id(broken_state_id);
        if !broken_block.is_air()
            && broken_state_id != new_state_id
            && broken_block != &Block::FIRE
            && broken_block != &Block::SOUL_FIRE
        {
            self.send_block_destroy_effect(position, broken_state_id, cause);
        }

        Some(broken_state_id)
    }

    fn prepare_observed_removal(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        expected: BlockStateId,
        flags: BlockFlags,
    ) -> Option<(BlockStateId, BlockFlags)> {
        let flags = self.observed_break_flags(position, cause, expected, flags)?;
        self.block_registry
            .player_will_destroy(self, cause, position, expected.to_state());
        let removal_state = self.get_block_state_id(position);
        // DecoratedPotBlock.playerWillDestroy marks its own state cracked before loot evaluation.
        let own_pot_crack = expected.to_block() == &Block::DECORATED_POT && {
            let mut properties =
                pumpkin_data::block_properties::DecoratedPotLikeProperties::from_state_id(expected);
            properties.cracked = true;
            properties.to_state_id(expected.to_block()) == removal_state
        };
        (removal_state == expected || own_pot_crack).then_some((removal_state, flags))
    }

    fn prepare_observed_block_loot(
        self: &Arc<Self>,
        position: &BlockPos,
        state: BlockStateId,
        cause: Option<&Arc<Player>>,
    ) -> super::observed_loot::PreparedBlockDrops {
        let tool = cause.and_then(|player| {
            let item = player.inventory().held_item();
            (!item.is_empty()).then_some(item)
        });
        let params = crate::world::loot::LootContextParameters {
            tool,
            block_state: Some(state.to_state()),
            position: Some(position.to_f64()),
            killed_by_player: Some(cause.is_some()),
            world: Some(self.clone()),
            registry: self
                .server
                .upgrade()
                .map(|server| server.datapack_manager.clone()),
            ..Default::default()
        };
        super::observed_loot::PreparedBlockDrops::prepare(self, position, state, &params)
    }
}
