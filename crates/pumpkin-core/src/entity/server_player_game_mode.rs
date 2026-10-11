use std::sync::{Arc, atomic::Ordering};

use pumpkin_data::{Block, BlockState, statistic::StatisticCategory};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::player::{MINE_BLOCK_EXHAUSTION, Player};
use crate::{net::ClientPlatform, server::Server, world::World};

// ServerPlayerGameMode.handleBlockBreakAction's STOP_DESTROY_BLOCK threshold.
const MIN_DESTROY_PROGRESS: f32 = 0.7;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DelayedDestroy {
    position: BlockPos,
    start: i32,
    lifecycle: u64,
}

impl Player {
    /// Matches an active dig against its admitted position, excluding delayed completion.
    pub(crate) fn is_destroying_block_at(&self, position: &BlockPos) -> bool {
        self.mining.load(Ordering::Relaxed)
            && self.mining_lifecycle.load(Ordering::Relaxed)
                == self.living_entity.damage_lifecycle()
            && *self
                .mining_pos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                == *position
    }

    pub(crate) fn finish_block_breaking(self: &Arc<Self>, position: &BlockPos, server: &Server) {
        // ServerPlayerGameMode.handleBlockBreakAction only finishes the current destroyPos.
        if !self.is_destroying_block_at(position) {
            return;
        }
        let world = self.world();
        if !self.can_interact_with_block_at(position, 1.0)
            || !self.may_break_block(&world, position)
            || !server
                .item_registry
                .can_mine(self.inventory().held_item().item, self)
        {
            self.stop_mining();
            return;
        }
        let (block, state) = world.get_block_and_state(position);
        if state.is_air() {
            self.stop_mining();
            return;
        }
        let elapsed = self
            .tick_counter
            .load(Ordering::Relaxed)
            .saturating_sub(self.start_mining_time.load(Ordering::Relaxed))
            .saturating_add(1);
        let progress = crate::block::calc_block_breaking(self, state, block) * elapsed as f32;
        if progress >= MIN_DESTROY_PROGRESS {
            self.stop_mining();
            self.destroy_mined_block(&world, position, state, server);
        } else {
            // Vanilla retains delayedDestroyPos until tick reaches full destruction progress.
            let mut delayed = self
                .delayed_destroy
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if delayed.is_none() {
                self.mining.store(false, Ordering::Relaxed);
                self.current_block_destroy_stage
                    .store(-1, Ordering::Relaxed);
                *delayed = Some(DelayedDestroy {
                    position: *position,
                    start: self.start_mining_time.load(Ordering::Relaxed),
                    lifecycle: self.mining_lifecycle.load(Ordering::Relaxed),
                });
            }
        }
    }

    pub(crate) fn tick_block_breaking(&self) {
        let delayed = *self
            .delayed_destroy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.mining.load(Ordering::Relaxed) && delayed.is_none() {
            return;
        }
        let world = self.world();
        let Some(player) = world.get_player_by_uuid(self.gameprofile.id) else {
            return;
        };
        let Some(server) = world.server.upgrade() else {
            return;
        };
        // ServerPlayerGameMode.tick advances delayedDestroyPos before the active destroyPos.
        if let Some(delayed) = delayed {
            self.tick_delayed_destroy(&player, &world, &server, delayed);
            return;
        }
        if !self.mining.load(Ordering::Relaxed) {
            return;
        }
        let position = *self
            .mining_pos
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = world.get_block_state(&position);
        // ServerPlayerGameMode.tick -> destroyBlock validates again for delayed destruction.
        if state.is_air()
            || !self.is_destroying_block_at(&position)
            || !self.can_interact_with_block_at(&position, 1.0)
            || !self.may_break_block(&world, &position)
            || !server
                .item_registry
                .can_mine(self.inventory().held_item().item, &player)
        {
            self.stop_mining();
            return;
        }
        let finished = self.continue_mining(
            position,
            &world,
            state,
            self.start_mining_time.load(Ordering::Relaxed),
        );
        if finished && matches!(self.client.as_ref(), ClientPlatform::Bedrock(_)) {
            self.stop_mining();
            player.destroy_mined_block(&world, &position, state, &server);
        }
    }

    fn tick_delayed_destroy(
        &self,
        player: &Arc<Self>,
        world: &Arc<World>,
        server: &Server,
        delayed: DelayedDestroy,
    ) {
        let state = world.get_block_state(&delayed.position);
        // PlayerList.respawn creates a fresh ServerPlayerGameMode; old digs cannot survive it.
        let authorized = delayed.lifecycle == self.living_entity.damage_lifecycle()
            && self.can_interact_with_block_at(&delayed.position, 1.0)
            && self.may_break_block(world, &delayed.position)
            && server
                .item_registry
                .can_mine(self.inventory().held_item().item, self);
        let finished = !state.is_air()
            && authorized
            && self.continue_mining(delayed.position, world, state, delayed.start);
        if state.is_air() || !authorized || finished {
            let mut pending = self
                .delayed_destroy
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *pending != Some(delayed) {
                return;
            }
            *pending = None;
            drop(pending);
            if !authorized {
                // ServerPlayerGameMode.tick publishes delayed progress to chunk observers.
                world.set_block_breaking(
                    &self.living_entity.entity,
                    delayed.position,
                    crate::world::BlockBreakingProgress::Stop,
                );
            }
            if finished {
                player.destroy_mined_block(world, &delayed.position, state, server);
            }
        }
    }

    // ServerPlayerGameMode.destroyBlock retains tool wear, exhaustion, callbacks and stats.
    fn destroy_mined_block(
        self: &Arc<Self>,
        world: &Arc<World>,
        position: &BlockPos,
        state: &BlockState,
        server: &Server,
    ) {
        let block = Block::from_state_id(state.id);
        let drops = !self.is_creative() && self.can_harvest(state, block);
        let flags = if drops {
            BlockFlags::NOTIFY_ALL
        } else {
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS
        };
        if world.break_block(position, Some(self), flags).is_some() {
            server
                .block_registry
                .broken(world, block, self, position, server, state);
            self.apply_tool_damage_for_block_break(state);
            if drops {
                self.add_exhaustion(MINE_BLOCK_EXHAUSTION);
            }
            let item_id = self.inventory().held_item().item.id;
            self.increment_stat(StatisticCategory::Used, i32::from(item_id), 1);
            self.increment_stat(StatisticCategory::Mined, i32::from(block.id.as_u16()), 1);
        }
    }
}
