use super::World;
use crate::{
    entity::{EntityBase, player::Player},
    plugin::player::{
        player_change_world::PlayerChangeWorldEvent, player_respawn::PlayerRespawnEvent,
        player_spawn_location::PlayerSpawnLocationEvent,
    },
    server::Server,
};
use pumpkin_data::dimension::Dimension;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::client::play::{CGameEvent, CPlayerSpawnPosition, CRespawn, GameEvent, PlayerSpawnData},
};
use pumpkin_util::{
    math::{
        boundingbox::BoundingBox, get_section_cord, position::BlockPos, vector2::Vector2,
        vector3::Vector3,
    },
    resource_location::ResourceLocation,
};
use pumpkin_world::biome;
use rayon::prelude::*;
use std::{sync::Arc, sync::atomic::Ordering};
use tracing::{debug, warn};

type RespawnLocation = (Option<Arc<Server>>, Arc<World>, Vector3<f64>, f32, f32);
type RespawnDestination = (Arc<World>, Vector3<f64>, f32, f32);
type PlayerTickCache<'a> = (
    &'a Arc<Player>,
    Vector3<f64>,
    BoundingBox,
    Vector2<i32>,
    u64,
);

impl World {
    pub(super) async fn respawn_player_inner(self: &Arc<Self>, player: &Arc<Player>, alive: bool) {
        let last_pos = player.get_entity().last_pos.load();
        let death_dimension = ResourceLocation::from(player.world().dimension.minecraft_name);
        let death_location = BlockPos(Vector3::new(
            last_pos.x.round() as i32,
            last_pos.y.round() as i32,
            last_pos.z.round() as i32,
        ));

        let data_kept = u8::from(alive);

        let Some((server, respawn_world, position, yaw, pitch)) =
            self.respawn_location(player).await
        else {
            return;
        };

        let Some(source_world) = player
            .living_entity
            .begin_respawn_after_transfer(player)
            .await
        else {
            return;
        };

        // PlayerList.respawn removes the old player's menus before transferring worlds.
        player.remove_respawn_menus(alive);

        let Some((target_world, position, yaw, pitch)) = Self::resolve_respawn_world(
            player,
            server.as_ref(),
            &source_world,
            (respawn_world, position, yaw, pitch),
        )
        .await
        else {
            return;
        };

        // PlayerList.respawn calls ServerPlayer.restoreFrom before publishing the new life.
        player.restore_inventory_after_respawn(alive);
        player.living_entity.reset_state();
        // ServerPlayer.restoreFrom starts a dead player's new FoodData before publication.
        if !alive {
            player.hunger_manager.restart();
        }

        // Notify plugins that the player has respawned (non-cancellable).
        if let Some(server) = source_world.server.upgrade() {
            server
                .plugin_manager
                .fire(
                    &server,
                    &mut PlayerRespawnEvent::new(
                        player.clone(),
                        source_world.clone(),
                        target_world.clone(),
                        position,
                        yaw,
                        pitch,
                        alive,
                    ),
                )
                .await;
        }

        if !player.living_entity.respawn_can_continue(player) {
            return;
        }

        target_world
            .send_respawn_packets(
                player,
                position,
                yaw,
                pitch,
                (death_dimension, death_location, data_kept),
            )
            .await;

        target_world
            .finish_respawn(player, &source_world, position, yaw, pitch)
            .await;
    }

    // PlayerList.respawn positions and publishes the restored player before tracking it.
    async fn finish_respawn(
        self: &Arc<Self>,
        player: &Arc<Player>,
        source_world: &Self,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
    ) {
        if !player
            .living_entity
            .complete_respawn(player, self, source_world, position, yaw, pitch)
            .await
        {
            return;
        }

        // Load chunks and send world info FIRST (before teleport packet)
        self.send_world_info(player);
        player.sync_respawn_inventory();
        self.send_center_chunk(player).await;

        // Send teleport packet after at least the center chunk was delivered
        // cancellation leaves the current position untouched.
        let _ = player.request_teleport(position, yaw, pitch);

        self.refresh_java_player_for_bedrock(player).await;
    }

    // ServerPlayer.findRespawnPositionAndUseSpawnBlock supplies PlayerList.respawn's destination.
    async fn respawn_location(self: &Arc<Self>, player: &Arc<Player>) -> Option<RespawnLocation> {
        let server = self.server.upgrade();
        let default_world = server.as_ref().map_or_else(
            || self.clone(),
            |s| s.get_world_from_dimension(&Dimension::OVERWORLD),
        );

        // Copy spawn info from default world level_info to avoid holding lock across await
        let (spawn_x, spawn_y, spawn_z, spawn_yaw, spawn_pitch) = {
            let info = default_world.level_info.load();
            (
                info.spawn_x,
                info.spawn_y,
                info.spawn_z,
                info.spawn_yaw,
                info.spawn_pitch,
            )
        };

        // ServerPlayer.findRespawnPositionAndUseSpawnBlock retains the actual destination level.
        let (position, yaw, pitch, respawn_world) = if let Some(respawn) =
            player.calculate_respawn_point().await
        {
            (respawn.position, respawn.yaw, respawn.pitch, respawn.world)
        } else {
            // No valid respawn point - send notification if player had one set
            if player
                .respawn_point
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
            {
                player
                    .send_client_packet(&CGameEvent::new(GameEvent::NoRespawnBlockAvailable, 0.0))
                    .await;
                let mut guard = player
                    .respawn_point
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(point) = guard.as_ref()
                    && !point.force
                {
                    *guard = None;
                }
            }

            // FIXME: This spawn position calculation is incorrect. Should use vanilla's
            // proper spawn position calculation (see #1381). The y-level calculation
            // needs to account for spawn radius and find a safe spawn position.
            let chunk_pos = Vector2::new(spawn_x >> 4, spawn_z >> 4);
            default_world
                .load_player_chunk(chunk_pos, player)
                .await
                .as_ref()?;
            let top = default_world.get_top_block(Vector2::new(spawn_x, spawn_z));
            let pos_y = if top > default_world.dimension.min_y {
                top + 1
            } else {
                spawn_y
            };

            (
                Vector3::new(
                    f64::from(spawn_x) + 0.5,
                    f64::from(pos_y),
                    f64::from(spawn_z) + 0.5,
                ),
                spawn_yaw,
                spawn_pitch,
                default_world.clone(),
            )
        };

        let mut spawn_loc_event = PlayerSpawnLocationEvent::new(player.clone(), position);
        if let Some(ref s) = server {
            s.plugin_manager.fire(s, &mut spawn_loc_event).await;
        }
        let position = spawn_loc_event.spawn_pos;
        if player.client.closed() || player.get_entity().is_removed() {
            return None;
        }

        Some((server, respawn_world, position, yaw, pitch))
    }

    // PlayerList.respawn removes the old life before adding it to TeleportTransition.newLevel.
    async fn resolve_respawn_world(
        player: &Arc<Player>,
        server: Option<&Arc<Server>>,
        source_world: &Arc<Self>,
        respawn: RespawnDestination,
    ) -> Option<RespawnDestination> {
        let (respawn_world, position, yaw, pitch) = respawn;
        // PlayerList.respawn uses TeleportTransition.newLevel, even for equal dimension types.
        let candidate_world = (respawn_world.uuid != source_world.uuid).then_some(respawn_world);

        // Fire PlayerChangeWorldEvent (cancellable) before the transfer; it runs before
        // the non-cancellable PlayerRespawnEvent, which observes the resolved world.
        let (resolved_world, position, yaw, pitch) = if let Some(new_world) = candidate_world {
            if let Some(s) = server {
                let mut event = PlayerChangeWorldEvent {
                    player: player.clone(),
                    previous_world: source_world.clone(),
                    new_world: new_world.clone(),
                    position,
                    yaw,
                    pitch,
                    cancelled: false,
                };
                s.plugin_manager.fire(s, &mut event).await;
                if !player.living_entity.respawn_can_continue(player) {
                    return None;
                }

                if event.cancelled {
                    (None, position, yaw, pitch)
                } else {
                    let destination = event.new_world;
                    let position = event.position;
                    let yaw = event.yaw;
                    let pitch = event.pitch;

                    // Skip the transfer if redirected back to the current world.
                    if destination.uuid != source_world.uuid {
                        debug!(
                            "Cross-dimension respawn: {} -> {}",
                            source_world.dimension.minecraft_name,
                            destination.dimension.minecraft_name
                        );

                        // Detach from the old world before publishing into the new one, so no
                        // observer sees the player in a world whose chunk manager doesn't match.
                        source_world.remove_player(player, false).await;
                        player.unload_watched_chunks(source_world).await;
                        let _owner = player.living_entity.own_damage();
                        if !player.living_entity.respawn_can_continue(player) {
                            return None;
                        }
                        player.change_world_chunks(&source_world.level, &destination);
                        player.living_entity.entity.set_world(destination.clone());
                    }

                    (Some(destination), position, yaw, pitch)
                }
            } else {
                warn!("Server dropped during cross-dimension respawn");
                (None, position, yaw, pitch)
            }
        } else {
            (None, position, yaw, pitch)
        };

        // Cancelled or unresolved cross-dimension respawns fall back to the current
        // world's spawn below; otherwise the resolved values from the event apply.
        let (target_world, position, yaw, pitch) = resolved_world.as_ref().map_or_else(
            || (source_world.clone(), position, yaw, pitch),
            |new_world| (new_world.clone(), position, yaw, pitch),
        );

        Some((target_world, position, yaw, pitch))
    }

    // PlayerList.respawn sends the destination dimension and default spawn position.
    async fn send_respawn_packets(
        &self,
        player: &Arc<Player>,
        position: Vector3<f64>,
        yaw: f32,
        pitch: f32,
        death: (ResourceLocation, BlockPos, u8),
    ) {
        let (death_dimension, death_location, data_kept) = death;
        // Send respawn packet with target dimension (using send_packet_now to ensure proper order)
        player
            .send_client_packet(&CRespawn::new(
                PlayerSpawnData::new(
                    self.dimension.clone(),
                    biome::hash_seed(self.level.seed.0),
                    player.gamemode.load() as u8,
                    player.gamemode.load() as i8,
                    false,
                    false,
                    Some((death_dimension, death_location)),
                    VarInt(player.get_entity().portal_cooldown.load(Ordering::Relaxed) as i32),
                    self.sea_level.into(),
                ),
                data_kept,
            ))
            .await;

        // Inform the client of the default spawn position so the client doesn't
        // fall back to (0, 2, 0) while the world reloads (fixes rubberbanding).
        // This must be sent after the CRespawn packet for proper client positioning.
        let spawn_block_pos = BlockPos(Vector3::new(
            position.x.round() as i32,
            position.y.round() as i32,
            position.z.round() as i32,
        ));
        let bedrock_dimension = match self.dimension.minecraft_name {
            "minecraft:the_nether" => 1,
            "minecraft:the_end" => 2,
            _ => 0,
        };
        player
            .send_packet_now_editioned(
                &CPlayerSpawnPosition::new(
                    spawn_block_pos,
                    yaw,
                    pitch,
                    self.dimension.minecraft_name.to_string(),
                ),
                &pumpkin_protocol::bedrock::client::CSetSpawnPosition {
                    spawn_position_type:
                        pumpkin_protocol::bedrock::client::SpawnPositionType::WorldRespawn,
                    block_position: spawn_block_pos,
                    dimension_type: bedrock_dimension.into(),
                    spawn_block_pos,
                },
            )
            .await;

        player.send_permission_lvl_update();
    }

    // PlayerList.respawn replaces the player; cached parallel ticks must retain the captured life.
    pub(super) fn players_cache_for_tick(players: &[Arc<Player>]) -> Vec<PlayerTickCache<'_>> {
        players
            .par_iter()
            .filter_map(|player| {
                let _owner = player.living_entity.own_damage();
                if player.living_entity.is_respawning() {
                    return None;
                }
                let entity = player.get_entity();
                let pos = entity.pos.load();
                let bb = entity.bounding_box.load().expand(1.0, 0.5, 1.0);
                let chunk_pos = Vector2::new(
                    get_section_cord(pos.x.floor() as i32),
                    get_section_cord(pos.z.floor() as i32),
                );
                Some((
                    player,
                    pos,
                    bb,
                    chunk_pos,
                    player.living_entity.damage_lifecycle(),
                ))
            })
            .collect()
    }
}
