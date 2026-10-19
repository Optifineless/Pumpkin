use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};
use pumpkin_util::{
    GameMode,
    math::{
        boundingbox::{BoundingBox, EntityDimensions},
        position::BlockPos,
        vector3::Vector3,
    },
};
use pumpkin_world::chunk::ChunkHeightmapType;
use rand::RngExt;

use super::{
    TeleportTransition,
    residency::{PortalChunkResidency, run_portal_job},
};
use crate::{
    entity::{
        living::random_teleport::blocks_teleport, player::Player, spawn_util::is_face_full_up,
    },
    world::World,
};

const ABSOLUTE_MAX_ATTEMPTS: i64 = 1024;
const COLLISION_EPSILON: f64 = 1.0e-7;

pub(super) async fn player_destination(
    source: &World,
    fallback: Arc<World>,
    player: &Arc<Player>,
) -> Option<TeleportTransition> {
    if !is_current(source, player) {
        return None;
    }
    let saved_spawn = player
        .respawn_point
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // EndPortalBlock -> ServerPlayer.findRespawnPositionAndUseSpawnBlock(false).
    let transition = if let Some((respawn, world)) =
        player.find_respawn_and_use_spawn_block(false).await
    {
        if world.level.cancel_token.is_cancelled() {
            return None;
        }
        TeleportTransition {
            new_world: world,
            position: respawn.position,
            yaw: Some(respawn.yaw),
            pitch: Some(respawn.pitch),
        }
    } else {
        let info = fallback.level_info.load_full();
        let suggestion = BlockPos::new(info.spawn_x, info.spawn_y, info.spawn_z);
        let position = find_adjusted_shared_spawn(&fallback, suggestion).await?;
        let unchanged = *player
            .respawn_point
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == saved_spawn;
        if !unchanged || !is_current(source, player) {
            return None;
        }
        let saved_world_available = saved_spawn.as_ref().is_some_and(|spawn| {
            source.server.upgrade().is_some_and(|server| {
                server.worlds.load().iter().any(|world| {
                    world.dimension == spawn.dimension && !world.level.cancel_token.is_cancelled()
                })
            })
        });
        if saved_world_available {
            // TeleportTransition.missingRespawnBlock notifies without clearing the saved record.
            player
                .try_send_client_packet(&CGameEvent::new(GameEvent::NoRespawnBlockAvailable, 0.0));
        }
        TeleportTransition {
            new_world: fallback,
            position,
            yaw: Some(info.spawn_yaw),
            pitch: Some(info.spawn_pitch),
        }
    };
    let unchanged = *player
        .respawn_point
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == saved_spawn;
    (unchanged && is_current(source, player)).then_some(transition)
}

pub(super) fn is_current(source: &World, player: &Player) -> bool {
    !source.level.cancel_token.is_cancelled()
        && !player.client.closed()
        && !player.living_entity.entity.is_removed()
        && std::ptr::eq(source, player.world().as_ref())
        && source
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|current| std::ptr::eq(current.as_ref(), player))
}

async fn find_adjusted_shared_spawn(
    world: &Arc<World>,
    suggestion: BlockPos,
) -> Option<Vector3<f64>> {
    let server = world.server.upgrade()?;
    let adventure = server
        .defaultgamemode
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .gamemode
        == GameMode::Adventure;
    if !adventure {
        let radius = search_radius(world, suggestion);
        let side = i64::from(radius) * 2 + 1;
        let count = side.saturating_mul(side).min(ABSOLUTE_MAX_ATTEMPTS);
        let coprime = if count <= 16 { count - 1 } else { 17 };
        let offset = rand::rng().random_range(0..count);
        // PlayerSpawnFinder.scheduleNext visits each bounded candidate in a coprime order.
        for index in 0..count {
            let value = (offset + coprime * index) % count;
            let x = suggestion
                .0
                .x
                .saturating_add((value % side) as i32 - radius);
            let z = suggestion
                .0
                .z
                .saturating_add((value / side) as i32 - radius);
            let mut residency = PortalChunkResidency::new(world.level.clone());
            residency.add_area(BlockPos::new(x, suggestion.0.y, z), 2);
            residency.load().await?;
            let scan_world = world.clone();
            let position = run_portal_job(world, move || {
                let _residency = residency;
                Some(get_level_respawn_pos(&scan_world, x, z))
            })
            .await?;
            if let Some(position) = position {
                return Some(position);
            }
        }
    }
    let mut residency = PortalChunkResidency::new(world.level.clone());
    residency.add_area(suggestion, 2);
    residency.load().await?;
    let scan_world = world.clone();
    run_portal_job(world, move || {
        let _residency = residency;
        Some(fixup_spawn_height(&scan_world, suggestion))
    })
    .await
}

fn search_radius(world: &World, suggestion: BlockPos) -> i32 {
    let radius = world.level_info.load().game_rules.respawn_radius.max(0);
    let distance = {
        let border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let half = border.new_diameter / 2.0;
        let x = f64::from(suggestion.0.x);
        let z = f64::from(suggestion.0.z);
        (x - (border.center_x - half))
            .min(border.center_x + half - x)
            .min(z - (border.center_z - half))
            .min(border.center_z + half - z)
            .floor() as i64
    };
    if distance <= 1 {
        1
    } else {
        radius.min(distance) as i32
    }
}

fn get_level_respawn_pos(world: &World, x: i32, z: i32) -> Option<Vector3<f64>> {
    // The End exit's shared respawn level is the Overworld. Pumpkin heightmaps store
    // the highest occupied Y, as LevelChunk.getHeight does. A fluid stops the descent.
    let top = world.get_heightmap_height(ChunkHeightmapType::MotionBlocking, x, z);
    // Pumpkin's cached motion map omits waterlogged nonmoving blocks; vanilla's
    // motion map includes their fluid. Reject those above the cached column too.
    let surface = world.get_heightmap_height(ChunkHeightmapType::WorldSurface, x, z);
    for y in (top + 1..=surface).rev() {
        if !world
            .get_fluid_and_fluid_state(&BlockPos::new(x, y, z))
            .1
            .is_empty
        {
            return None;
        }
    }
    for y in (world.min_y..=top + 1).rev() {
        let pos = BlockPos::new(x, y, z);
        let state = world.get_block_state(&pos);
        if !world.get_fluid_and_fluid_state(&pos).1.is_empty {
            return None;
        }
        if is_face_full_up(world, &pos, state) {
            return no_collision_no_liquid(world, pos.up()).then(|| bottom_center(pos.up()));
        }
    }
    None
}

fn fixup_spawn_height(world: &World, mut pos: BlockPos) -> Vector3<f64> {
    // PlayerSpawnFinder.fixupSpawnHeight adjusts the suggestion, never the saved SpawnY.
    while !no_collision_no_liquid(world, pos) && pos.0.y < world.min_y + world.dimension.height - 1
    {
        pos = pos.up();
    }
    pos = pos.down();
    while no_collision_no_liquid(world, pos) && pos.0.y > world.min_y {
        pos = pos.down();
    }
    bottom_center(pos.up())
}

fn no_collision_no_liquid(world: &World, pos: BlockPos) -> bool {
    let kind = &EntityType::PLAYER;
    let dimensions = EntityDimensions::new(kind.width(), kind.height(), kind.eye_height);
    let center = bottom_center(pos);
    let bounds = BoundingBox::new_from_pos(center.x, center.y, center.z, &dimensions);
    for cell in BlockPos::iterate(
        bounds.min_block_pos().add(-1, -1, -1),
        bounds.max_block_pos().add(1, 1, 1),
    ) {
        if !world.is_in_height_limit(cell.0.y) {
            continue;
        }
        let Some(state) = world.get_block_state_if_loaded(&cell) else {
            return false;
        };
        let mut collision = false;
        world.for_each_collision_shape(state, &cell, None, &mut |shape| {
            collision |= shape.at_pos(cell).intersects(&bounds);
        });
        if collision
            || (state.is_liquid()
                && BoundingBox::new_array([0.0; 3], [1.0; 3])
                    .at_pos(cell)
                    .intersects(&bounds))
        {
            return false;
        }
    }
    !world
        .get_entities_at_box(&bounds.expand(
            COLLISION_EPSILON,
            COLLISION_EPSILON,
            COLLISION_EPSILON,
        ))
        .iter()
        .any(|entity| blocks_teleport(entity.as_ref()))
}

fn bottom_center(pos: BlockPos) -> Vector3<f64> {
    Vector3::new(
        f64::from(pos.0.x) + 0.5,
        f64::from(pos.0.y),
        f64::from(pos.0.z) + 0.5,
    )
}

// Keep the existing credits packet and its ordering before destination preparation.
pub(super) fn send_credits(current_level: &World, caller: &dyn crate::entity::EntityBase) {
    if let Some(player) = current_level.get_player_by_id(caller.get_entity().entity_id) {
        match player.client.as_ref() {
            crate::net::ClientPlatform::Java(client) => {
                if let Ok(data) =
                    client.serialize_packet(&pumpkin_protocol::java::client::play::CGameEvent::new(
                        pumpkin_protocol::java::client::play::GameEvent::WinGame,
                        1.0,
                    ))
                {
                    client.try_enqueue_packet(data);
                }
            }
            crate::net::ClientPlatform::Bedrock(client) => {
                if let Ok(data) =
                    client.serialize_packet(&pumpkin_protocol::bedrock::client::CShowCredits {
                        player_runtime_id: (caller.get_entity().entity_id as u64).into(),
                        credits_state: 0.into(),
                    })
                {
                    client.try_enqueue_packet(data);
                }
            }
        }
    }
}
