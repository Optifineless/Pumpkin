use super::World;
use crate::{block::OnNeighborUpdateArgs, entity::player::Player};
use pumpkin_data::{Block, BlockDirection, BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex};
use tracing::error;

// LevelAccessor.setBlock and Block.updateOrDestroy default shape depth.
pub(crate) const UPDATE_LIMIT: u32 = 512;

#[derive(Clone, Copy)]
enum SingleUpdate {
    Neighbor {
        position: BlockPos,
        source_position: BlockPos,
        source_block: BlockId,
        include_fluid: bool,
    },
    Shape {
        position: BlockPos,
        direction: BlockDirection,
        neighbor_position: BlockPos,
        neighbor_state_id: BlockStateId,
        flags: BlockFlags,
        update_limit: u32,
    },
}

#[derive(Clone, Copy)]
enum UpdateKind {
    Single(SingleUpdate),
    Multi {
        position: BlockPos,
        source_block: BlockId,
        except: Option<BlockDirection>,
        index: usize,
    },
}
impl UpdateKind {
    const fn position(&self) -> BlockPos {
        match self {
            Self::Single(
                SingleUpdate::Neighbor { position, .. } | SingleUpdate::Shape { position, .. },
            )
            | Self::Multi { position, .. } => *position,
        }
    }
}

/// A world's resumable CollectingNeighborUpdater cascade; locks are released before callbacks.
#[derive(Default)]
pub(super) struct UpdateStack {
    remaining: Vec<UpdateKind>,
    staged: Vec<UpdateKind>,
    submitted: u64,
    running: bool,
}

impl UpdateStack {
    fn stage(&mut self, kind: UpdateKind, limit: i32) {
        // CollectingNeighborUpdater.addAndRun counts a MultiNeighborUpdate once.
        let too_many = limit >= 0 && self.submitted >= limit as u64;
        if !too_many {
            self.staged.push(kind);
        } else if self.submitted == limit as u64 {
            error!("Too many chained neighbor updates at {}", kind.position());
        }
        self.submitted = self.submitted.saturating_add(1);
    }

    fn next(&mut self) -> Option<SingleUpdate> {
        self.remaining.extend(self.staged.drain(..).rev());
        match self.remaining.pop()? {
            UpdateKind::Multi {
                position,
                source_block,
                except,
                mut index,
            } => {
                let order = BlockDirection::update_order();
                while index < order.len() && Some(order[index]) == except {
                    index += 1;
                }
                let direction = *order.get(index)?;
                index += 1;
                while index < order.len() && Some(order[index]) == except {
                    index += 1;
                }
                if index < order.len() {
                    self.remaining.push(UpdateKind::Multi {
                        position,
                        source_block,
                        except,
                        index,
                    });
                }
                Some(SingleUpdate::Neighbor {
                    position: position.offset(direction.to_offset()),
                    source_position: position,
                    source_block,
                    include_fluid: true,
                })
            }
            UpdateKind::Single(next) => Some(next),
        }
    }
}

struct CascadeGuard<'a> {
    stack: &'a Mutex<UpdateStack>,
    armed: bool,
}
impl Drop for CascadeGuard<'_> {
    fn drop(&mut self) {
        // CollectingNeighborUpdater.runUpdates finally clears state after callback panics.
        if self.armed {
            *self
                .stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = UpdateStack::default();
        }
    }
}

pub(super) fn update_neighbors_at(
    world: &Arc<World>,
    position: &BlockPos,
    source_block: &Block,
    except: Option<BlockDirection>,
) {
    submit(
        world,
        UpdateKind::Multi {
            position: *position,
            source_block: source_block.id,
            except,
            index: 0,
        },
    );
}
pub(super) fn update_neighbor(world: &Arc<World>, position: &BlockPos, source_block: &Block) {
    submit(
        world,
        UpdateKind::Single(SingleUpdate::Neighbor {
            position: *position,
            source_position: *position,
            source_block: source_block.id,
            include_fluid: false,
        }),
    );
}
pub(super) fn update_shape(
    world: &Arc<World>,
    position: &BlockPos,
    direction: BlockDirection,
    flags: BlockFlags,
    update_limit: u32,
) {
    let neighbor_position = position.offset(direction.to_offset());
    // CollectingNeighborUpdater.ShapeUpdate captures the source state when enqueued.
    let neighbor_state_id = world.get_block_state_id(&neighbor_position);
    submit(
        world,
        UpdateKind::Single(SingleUpdate::Shape {
            position: *position,
            direction,
            neighbor_position,
            neighbor_state_id,
            flags,
            update_limit,
        }),
    );
}

fn submit(world: &Arc<World>, kind: UpdateKind) {
    let limit = world.server.upgrade().map_or_else(
        pumpkin_config::world::default_max_chained_neighbor_updates,
        |s| s.advanced_config.world.max_chained_neighbor_updates,
    );
    {
        let mut stack = world
            .neighbor_updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stack.stage(kind, limit);
        if stack.running {
            return;
        }
        stack.running = true;
    }
    let mut guard = CascadeGuard {
        stack: &world.neighbor_updates,
        armed: true,
    };
    loop {
        let next = {
            let mut stack = world
                .neighbor_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let next = stack.next();
            if next.is_none() {
                // Finish under the submission lock so a concurrent new cascade is never discarded.
                *stack = UpdateStack::default();
                guard.armed = false;
            }
            next
        };
        let Some(next) = next else {
            break;
        };
        execute(world, next);
    }
}

fn execute(world: &Arc<World>, kind: SingleUpdate) {
    match kind {
        SingleUpdate::Neighbor {
            position,
            source_position,
            source_block,
            include_fluid,
        } => {
            world.execute_neighbor_update(
                &position,
                &source_position,
                source_block.to_block(),
                include_fluid,
            );
        }
        SingleUpdate::Shape {
            position,
            direction,
            neighbor_position,
            neighbor_state_id,
            flags,
            update_limit,
        } => {
            world.execute_shape_update(
                &position,
                direction,
                &neighbor_position,
                neighbor_state_id,
                flags,
                update_limit,
            );
        }
    }
}

// Mirrors NeighborUpdater.executeUpdate, retaining the fork's cancellable physics event.
impl World {
    pub fn break_block(
        self: &Arc<Self>,
        position: &BlockPos,
        cause: Option<&Arc<Player>>,
        flags: BlockFlags,
    ) -> Option<BlockStateId> {
        self.break_block_with_limit(position, cause, flags, UPDATE_LIMIT)
    }

    // Level.setBlock's explicit depth argument is retained through shape replacements.
    pub(crate) fn set_block_state_with_limit(
        self: &Arc<Self>,
        position: &BlockPos,
        state: BlockStateId,
        flags: BlockFlags,
        update_limit: u32,
    ) -> BlockStateId {
        if !self.is_in_build_limit(*position) {
            return BlockStateId::AIR;
        }
        let old = self
            .write_block_state_if(position, state, |_| true)
            .unwrap_or(BlockStateId::AIR);
        self.on_block_state_set(position, old, state, flags, update_limit)
    }

    /// Queues a shape update with the remaining vanilla propagation depth.
    pub(crate) fn replace_with_state_for_neighbor_update_with_limit(
        self: &Arc<Self>,
        position: &BlockPos,
        direction: BlockDirection,
        flags: BlockFlags,
        update_limit: u32,
    ) {
        update_shape(self, position, direction, flags, update_limit);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Mirrors NeighborUpdater.executeShapeUpdate including its depth limit"
    )]
    fn execute_shape_update(
        self: &Arc<Self>,
        block_pos: &BlockPos,
        direction: BlockDirection,
        neighbor_pos: &BlockPos,
        neighbor_state_id: BlockStateId,
        flags: BlockFlags,
        update_limit: u32,
    ) {
        let (block, block_state_id) = self.get_block_and_state_id(block_pos);

        if flags.contains(BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT)
            && *block == Block::REDSTONE_WIRE
        {
            return;
        }

        let new_state_id = self.block_registry.get_state_for_neighbor_update(
            self,
            block,
            block_state_id,
            block_pos,
            direction,
            neighbor_pos,
            neighbor_state_id,
        );

        if new_state_id != block_state_id {
            if new_state_id.to_state().is_air() {
                self.break_block_with_limit(
                    block_pos,
                    None,
                    flags | BlockFlags::NOTIFY_ALL,
                    update_limit,
                );
            } else {
                self.set_block_state_with_limit(
                    block_pos,
                    new_state_id,
                    flags - BlockFlags::SKIP_DROPS,
                    update_limit,
                );
            }
        }
    }

    fn execute_neighbor_update(
        self: &Arc<Self>,
        position: &BlockPos,
        source_position: &BlockPos,
        source_block: &Block,
        include_fluid: bool,
    ) {
        let (block, fluid) = if include_fluid {
            let (block, fluid) = self.get_block_and_fluid(position);
            (block, Some(fluid))
        } else {
            (self.get_block(position), None)
        };

        let mut event = crate::plugin::api::events::block::block_physics::BlockPhysicsEvent::new(
            *position,
            *source_position,
        );
        if let Some(server) = self.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return;
        }

        if let Some(pumpkin_block) = self.block_registry.get_pumpkin_block(block.id) {
            pumpkin_block.on_neighbor_update(OnNeighborUpdateArgs {
                world: self,
                block,
                position,
                source_block,
                notify: false,
            });
        }

        if let Some(fluid) = fluid
            && let Some(pumpkin_fluid) = self.block_registry.get_pumpkin_fluid(fluid.id)
        {
            pumpkin_fluid.on_neighbor_update(self, fluid, position, false);
        }
    }
}

#[cfg(test)]
#[path = "neighbor_updater_tests.rs"]
mod tests;
