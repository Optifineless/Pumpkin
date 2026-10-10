use super::*;
use crate::{
    block::BlockHitResult,
    entity::{death_test_world::DeathTestWorld, player::Player},
    net::{ClientPlatform, GameProfile, PlayerConfig, java::JavaClient},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            block::block_break::BlockBreakEvent, entity::entity_explode::EntityExplodeEvent,
        },
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use arc_swap::ArcSwap;
use pumpkin_data::{
    Block, BlockDirection, biome::Biome, block_properties::RespawnAnchorLikeProperties,
    dimension::Dimension,
};
use pumpkin_util::{GameMode, math::position::BlockPos, math::vector3::Vector3};
use pumpkin_world::{cylindrical_chunk_iterator::Cylindrical, world::BlockFlags};
use std::num::NonZero;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};
use uuid::Uuid;

fn charged_state(charges: u8) -> pumpkin_data::BlockStateId {
    let mut properties =
        RespawnAnchorLikeProperties::from_state_id(Block::RESPAWN_ANCHOR.default_state.id);
    properties.charges = charges;
    properties.to_state_id(&Block::RESPAWN_ANCHOR)
}

fn use_anchor(
    fixture: &DeathTestWorld,
    world: &Arc<crate::world::World>,
    player: &Arc<Player>,
    position: &BlockPos,
) -> BlockActionResult {
    let cursor = Vector3::new(0.5, 1.0, 0.5);
    let hit = BlockHitResult {
        face: &BlockDirection::Up,
        cursor_pos: &cursor,
    };
    fixture.server.block_registry.on_use(
        &Block::RESPAWN_ANCHOR,
        player,
        position,
        &hit,
        &fixture.server,
        world,
    )
}

struct CancelBreak;
impl EventHandler<BlockBreakEvent> for CancelBreak {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

struct ExplosionCount(Arc<AtomicUsize>);
impl EventHandler<EntityExplodeEvent> for ExplosionCount {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut EntityExplodeEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

fn player_in_world(world: &Arc<crate::world::World>, name: &str) -> Arc<Player> {
    let profile = GameProfile {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        properties: ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    };
    let client = Arc::new(ClientPlatform::Java(JavaClient::without_connection(
        profile.clone(),
    )));
    let player = Arc::new(Player::new(
        client,
        profile,
        PlayerConfig::default(),
        world,
        GameMode::Survival,
    ));
    player.set_client_loaded(true);
    player
}

fn publish_air_chunk_for_dimension(world: &crate::world::World, biome: &Biome) {
    use pumpkin_util::world_seed::Seed;
    use pumpkin_world::{
        chunk::{ChunkLight, format::LightContainer},
        chunk_system::chunk_state::Chunk,
        generation::{
            generator::{WorldGenerator, flat::FlatGenerator},
            proto_chunk::ProtoChunk,
        },
    };

    let dimension = world.dimension.clone();
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        dimension.clone(),
        Vec::new(),
        biome.registry_id.to_owned(),
    )));
    let mut proto = ProtoChunk::new(0, 0, &generator);
    proto.flat_biome_map.fill(biome.id);
    proto.light = ChunkLight {
        sky_light: vec![LightContainer::new_empty(15); dimension.height as usize / 16]
            .into_boxed_slice(),
        block_light: vec![LightContainer::new_empty(0); dimension.height as usize / 16]
            .into_boxed_slice(),
    };
    let mut chunk = Chunk::Proto(Box::new(proto));
    chunk.upgrade_to_level_chunk(
        &dimension,
        &pumpkin_config::lighting::LightingEngineConfig::default(),
    );
    if let Chunk::Level(chunk) = chunk {
        world
            .level
            .loaded_chunks
            .insert(pumpkin_util::math::vector2::Vector2::new(0, 0), chunk);
    }
}

#[test]
fn item_use_matches_anchor_glowstone_and_hand_fallbacks() {
    assert_eq!(item_use_action(true, 3, true, false), ItemUseAction::Charge);
    assert_eq!(
        item_use_action(false, 3, true, true),
        ItemUseAction::PassToOffHand
    );
    assert_eq!(
        item_use_action(false, 3, false, true),
        ItemUseAction::UseWithoutItem
    );
    assert_eq!(
        item_use_action(false, 0, true, false),
        ItemUseAction::UseWithoutItem
    );
    assert_eq!(
        item_use_action(true, 4, true, true),
        ItemUseAction::UseWithoutItem
    );
}

#[test]
fn empty_hand_uses_only_charged_anchors() {
    assert_eq!(empty_hand_action(0, true), EmptyHandAction::Pass);
    assert_eq!(empty_hand_action(0, false), EmptyHandAction::Pass);
    assert_eq!(empty_hand_action(1, true), EmptyHandAction::SetSpawn);
    assert_eq!(empty_hand_action(4, false), EmptyHandAction::Explode);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_use_sets_spawn_once_in_nether() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    assert_eq!(
        world.dimension.minecraft_name,
        Dimension::THE_NETHER.minecraft_name
    );
    publish_air_chunk_for_dimension(&world, &Biome::NETHER_WASTES);
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(&position, charged_state(1), BlockFlags::FORCE_STATE);
    let player = player_in_world(&world, "AnchorSpawnTest");

    assert!(matches!(
        use_anchor(&fixture, &world, &player, &position),
        BlockActionResult::SuccessServer
    ));
    let spawn = player
        .respawn_point
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        spawn.is_some(),
        "the callback should set the player's spawn"
    );
    let Some(spawn) = spawn else { return };
    assert_eq!(spawn.dimension, Dimension::THE_NETHER);
    assert_eq!(spawn.position, position);
    assert_eq!(spawn.yaw, 0.0);

    assert!(
        matches!(
            use_anchor(&fixture, &world, &player, &position),
            BlockActionResult::Consume
        ),
        "using the already-selected anchor should not request a server hand swing"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_use_removes_anchor_with_client_notification() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let player = fixture.player("AnchorRemovalTest");
    player.watched_section.store(Cylindrical::new(
        pumpkin_util::math::vector2::Vector2::new(0, 0),
        NonZero::new(2).unwrap_or(NonZero::<u8>::MIN),
    ));
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(&position, charged_state(1), BlockFlags::FORCE_STATE);

    assert!(matches!(
        use_anchor(&fixture, &world, &player, &position),
        BlockActionResult::SuccessServer
    ));
    assert_eq!(world.get_block(&position), &Block::AIR);
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return;
    };
    let pending_before_flush = client.pending_bytes.load(Relaxed);
    world.flush_block_updates();
    assert!(
        client.pending_bytes.load(Relaxed) > pending_before_flush,
        "notifying removal should queue a block update for watching clients"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_anchor_removal_keeps_anchor_and_skips_explosion() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(&position, charged_state(1), BlockFlags::FORCE_STATE);
    let explosions = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(Arc::new(CancelBreak), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<EntityExplodeEvent, _>(
            Arc::new(ExplosionCount(explosions.clone())),
            EventPriority::Normal,
            true,
        );
    let player = fixture.player("AnchorCancelTest");

    assert!(matches!(
        use_anchor(&fixture, &world, &player, &position),
        BlockActionResult::SuccessServer
    ));
    assert_eq!(world.get_block(&position), &Block::RESPAWN_ANCHOR);
    assert_eq!(explosions.load(Relaxed), 0);
    fixture.server.shutdown().await;
}
