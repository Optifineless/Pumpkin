use super::support::*;
use crate::{
    entity::{death_test_world::DeathTestWorld, player::Player},
    net::ClientPlatform,
    server::Server,
    world::{World, projectile_test_support},
};
use pumpkin_data::{Block, biome::Biome, dimension::Dimension, entity::EntityType, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, Status},
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_trident_world_snapshot_transfer_rejects_publication() {
    let f = tokio::time::timeout(Duration::from_secs(60), DeathTestWorld::new())
        .await
        .unwrap();
    let mut p = player(&f);
    let origin = f.world();
    let destination = f.server.get_world_from_dimension(&Dimension::THE_NETHER);
    publish_destination(&destination);
    let mut trident = stack(1, &Item::TRIDENT);
    trident.set_damage(7);
    p.player.inventory().set_stack(0, trident.clone());
    use_item(&p, &f, Hand::Right);
    let hooks = Hooks::install(&f.server, &p.player);
    hooks.configure(Veto::None, false, None);
    hooks.state.lock().unwrap().check_reservations = true;
    let transferred = Arc::new(AtomicBool::new(false));
    let recv = release_with_transfer(
        p.player.clone(),
        f.server.clone(),
        destination.clone(),
        transferred.clone(),
    );
    let result = tokio::time::timeout(Duration::from_secs(60), p.with_outgoing_writer(recv))
        .await
        .unwrap()
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    assert!(
        transferred.load(SeqCst),
        "release missed the world-snapshot seam"
    );
    assert!(Arc::ptr_eq(&p.player.world(), &destination));
    assert!(!origin.contains_player_instance(&p.player));
    assert!(destination.contains_player_instance(&p.player));
    for world in [&origin, &destination] {
        assert!(
            world
                .entities
                .load()
                .iter()
                .all(|entity| entity.get_entity().entity_type != &EntityType::TRIDENT),
            "a candidate prepared in the old world was published after transfer"
        );
        assert_reservations_released(world.clone());
    }
    let actual = p.player.inventory().get_stack(0);
    assert_eq!(actual.uid, trident.uid);
    assert!(
        actual.are_equal(&trident),
        "world mismatch consumed or damaged the trident"
    );
    assert_eq!(
        hooks.counts(),
        (1, 1, 0),
        "the real release did not reach insertion after transfer"
    );
    assert_eq!(used(&p.player, &Item::TRIDENT), 0);
    assert_owner_released(p.player.inventory().slot_transaction(0));
    tokio::time::timeout(Duration::from_secs(60), f.server.shutdown())
        .await
        .unwrap();
}

fn release_with_transfer(
    player: Arc<Player>,
    server: Arc<Server>,
    destination: Arc<World>,
    transferred: Arc<AtomicBool>,
) -> tokio::sync::oneshot::Receiver<std::thread::Result<()>> {
    let runtime = tokio::runtime::Handle::current();
    let (send, recv) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let current = player.clone();
            let transition_runtime = runtime.clone();
            let _registration = projectile_test_support::install_world_snapshot(
                &player,
                move |origin, observed| {
                    assert!(std::ptr::eq(observed, current.as_ref()));
                    assert!(Arc::ptr_eq(origin, &current.world()));
                    assert_owner_released(current.inventory().slot_transaction(0));
                    assert_reservations_released(origin.clone());
                    transition_runtime.block_on(async {
                        tokio::time::timeout(
                            Duration::from_secs(30),
                            current.teleport_world(
                                destination.clone(),
                                Vector3::new(8.5, 64.0, 8.5),
                                Some(0.0),
                                Some(0.0),
                            ),
                        )
                        .await
                        .unwrap();
                    });
                    assert!(
                        Arc::ptr_eq(&current.world(), &destination),
                        "real dimension transfer did not commit"
                    );
                    current.set_client_loaded(true);
                    transferred.store(true, SeqCst);
                },
            );
            player.living_entity.item_use_time.store(72000 - 30, SeqCst);
            if let ClientPlatform::Java(client) = player.client.as_ref() {
                client.handle_player_action(
                    &player,
                    &SPlayerAction {
                        status: VarInt(Status::ReleaseItemInUse as i32),
                        position: BlockPos::new(0, 0, 0),
                        face: 0,
                        sequence: VarInt(2),
                    },
                    &server,
                );
            }
        }));
        let _ = send.send(result);
    });
    recv
}

fn publish_destination(world: &Arc<World>) {
    let dimension = Dimension::THE_NETHER;
    let generator = pumpkin_world::generation::generator::WorldGenerator::Flat(Box::new(
        pumpkin_world::generation::generator::flat::FlatGenerator::new(
            pumpkin_util::world_seed::Seed(0),
            dimension.clone(),
            Vec::new(),
            Biome::NETHER_WASTES.registry_id.to_owned(),
        ),
    ));
    let mut proto = pumpkin_world::generation::proto_chunk::ProtoChunk::new(0, 0, &generator);
    for x in 0..16 {
        for z in 0..16 {
            proto.set_block_state(x, 63, z, Block::STONE.default_state);
        }
    }
    let mut chunk = pumpkin_world::chunk_system::chunk_state::Chunk::Proto(Box::new(proto));
    chunk.upgrade_to_level_chunk(
        &dimension,
        &pumpkin_config::lighting::LightingEngineConfig::default(),
    );
    if let pumpkin_world::chunk_system::chunk_state::Chunk::Level(chunk) = chunk {
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    }
}
