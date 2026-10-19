use super::{CalculatedRespawnPoint, Player, RespawnPoint, Vector2, Vector3, debug, tag};
use crate::world::World;
use pumpkin_data::{Block, tag::Taggable};
use pumpkin_world::chunk_system::residency::ChunkResidency;
use std::sync::Arc;

impl Player {
    // ServerPlayer.findRespawnAndUseSpawnBlock: End returns inspect without paying a charge.
    pub(crate) async fn find_respawn_and_use_spawn_block(
        &self,
        consume_spawn_block: bool,
    ) -> Option<(CalculatedRespawnPoint, Arc<World>)> {
        let respawn_point = {
            let respawn_guard = self
                .respawn_point
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            respawn_guard.clone()?
        };
        let world = if self.world().dimension == respawn_point.dimension {
            self.world()
        } else if let Some(server) = self.world().server.upgrade() {
            if consume_spawn_block {
                server.get_world_from_dimension(&respawn_point.dimension)
            } else {
                server
                    .worlds
                    .load()
                    .iter()
                    .find(|world| {
                        world.dimension == respawn_point.dimension
                            && !world.level.cancel_token.is_cancelled()
                    })
                    .cloned()?
            }
        } else {
            self.world()
        };
        let pos = &respawn_point.position;

        // Ensure chunks around the spawn position are fetched
        let min_chunk_x = (pos.0.x - 2) >> 4;
        let max_chunk_x = (pos.0.x + 2) >> 4;
        let min_chunk_z = (pos.0.z - 2) >> 4;
        let max_chunk_z = (pos.0.z + 2) >> 4;
        let mut residency = ChunkResidency::new(world.level.chunk_loading.clone());
        residency.add_all(
            (min_chunk_x..=max_chunk_x)
                .flat_map(|x| (min_chunk_z..=max_chunk_z).map(move |z| Vector2::new(x, z))),
        );
        for cx in min_chunk_x..=max_chunk_x {
            for cz in min_chunk_z..=max_chunk_z {
                world
                    .level
                    .get_or_fetch_chunk(Vector2::new(cx, cz), |_| ())
                    .await
                    .ok()?;
            }
        }

        let result = Self::resolve_respawn_position(&world, &respawn_point, consume_spawn_block)?;
        Some((result, world))
    }

    fn resolve_respawn_position(
        world: &Arc<World>,
        respawn_point: &RespawnPoint,
        consume_spawn_block: bool,
    ) -> Option<CalculatedRespawnPoint> {
        type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;
        type AnchorProperties = pumpkin_data::block_properties::RespawnAnchorLikeProperties;
        let pos = &respawn_point.position;
        let (block, state_id) = world.get_block_and_state_id(pos);

        // If force is set (from /spawnpoint command), validate position is safe
        if respawn_point.force {
            // For forced spawn, check if both the block and block above allow mob spawn
            let block_state = world.get_block_state(pos);
            let above_state = world.get_block_state(&pos.up());

            // Check if blocks are passable (non-solid or air)
            let block_safe = block_state.is_air() || !block_state.is_solid();
            let above_safe = above_state.is_air() || !above_state.is_solid();

            if block_safe && above_safe {
                let position = Vector3::new(
                    f64::from(pos.0.x) + 0.5,
                    f64::from(pos.0.y) + 0.1,
                    f64::from(pos.0.z) + 0.5,
                );
                debug!(
                    "Returning forced spawn point at {:?}, dimension: {:?}",
                    position, respawn_point.dimension
                );
                return Some(CalculatedRespawnPoint {
                    position,
                    yaw: respawn_point.yaw,
                    pitch: 0.0,
                    dimension: respawn_point.dimension.clone(),
                });
            }
            return None;
        }

        // Handle bed respawn
        if block.has_tag(&tag::Block::MINECRAFT_BEDS) {
            let bed_props = BedProperties::from_state_id(state_id);
            let facing = bed_props.facing;

            // Try positions around the bed based on facing direction
            // Vanilla tries multiple offset patterns; we use a simplified version
            if let Some(spawn_pos) = Self::find_bed_spawn_position(world, pos, facing) {
                return Some(CalculatedRespawnPoint {
                    position: spawn_pos,
                    yaw: respawn_point.yaw,
                    pitch: 0.0,
                    dimension: respawn_point.dimension.clone(),
                });
            }
            return None;
        }

        // Handle respawn anchor (Nether)
        if block == &Block::RESPAWN_ANCHOR {
            let anchor_props = AnchorProperties::from_state_id(state_id);
            let charges = anchor_props.charges;

            // Anchor needs at least 1 charge to work
            if charges == 0 {
                return None;
            }

            // Try positions around the anchor
            if let Some(spawn_pos) = Self::find_anchor_spawn_position(world, pos) {
                if consume_spawn_block {
                    // Decrement charges after successful respawn position found
                    let new_charges = charges - 1;
                    let mut new_props = anchor_props;
                    new_props.charges = new_charges;
                    world.set_block_state(
                        pos,
                        new_props.to_state_id(block),
                        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
                    );
                }

                return Some(CalculatedRespawnPoint {
                    position: spawn_pos,
                    yaw: respawn_point.yaw,
                    pitch: 0.0,
                    dimension: respawn_point.dimension.clone(),
                });
            }
            return None;
        }

        None
    }
}
