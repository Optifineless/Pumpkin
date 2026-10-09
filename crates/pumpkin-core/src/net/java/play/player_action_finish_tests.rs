use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    net::java::combat_test_support::TestPlayer,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use pumpkin_util::permission::PermissionLvl;

fn action(player: &TestPlayer, server: &Server, status: Status, position: BlockPos) {
    player.client().handle_player_action(
        &player.player,
        &SPlayerAction {
            status: VarInt(status as i32),
            position,
            face: 1,
            sequence: VarInt(1),
        },
        server,
    );
}

async fn fixture() -> (DeathTestWorld, TestPlayer, BlockPos) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let pos = BlockPos::new(8, 63, 8);
    // Player.getDestroySpeed applies an airborne penalty until onGround is set.
    player
        .player
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    (fixture, player, pos)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_finish_only_rejects_spectator_adventure_border_and_unstarted_targets() {
    let (fixture, player, pos) = fixture().await;
    let world = fixture.world();
    for mode in [GameMode::Spectator, GameMode::Adventure, GameMode::Survival] {
        player.player.set_gamemode(mode);
        player.player.set_client_loaded(true);
        action(&player, &fixture.server, Status::FinishedDigging, pos);
        assert_eq!(
            world.get_block(&pos),
            &Block::STONE,
            "FINISH only in {mode:?}"
        );
    }
    world.worldborder.lock().unwrap().new_diameter = 2.0;
    action(&player, &fixture.server, Status::FinishedDigging, pos);
    assert_eq!(world.get_block(&pos), &Block::STONE);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_finish_rechecks_permissions_and_matching_target() {
    let (fixture, player, pos) = fixture().await;
    let world = fixture.world();
    action(&player, &fixture.server, Status::StartedDigging, pos);
    player.player.tick_counter.store(200, Ordering::Relaxed);
    action(
        &player,
        &fixture.server,
        Status::FinishedDigging,
        pos.offset(Vector3::new(1, 0, 0)),
    );
    assert_eq!(
        world.get_block(&pos.offset(Vector3::new(1, 0, 0))),
        &Block::STONE
    );
    assert!(player.player.mining.load(Ordering::Relaxed));
    player.player.set_gamemode(GameMode::Adventure);
    player.player.set_client_loaded(true);
    action(&player, &fixture.server, Status::FinishedDigging, pos);
    assert_eq!(world.get_block(&pos), &Block::STONE);
    assert!(!player.player.mining.load(Ordering::Relaxed));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_early_finish_keeps_delayed_target_after_abort_and_new_start() {
    let (fixture, player, pos) = fixture().await;
    let world = fixture.world();
    action(&player, &fixture.server, Status::StartedDigging, pos);
    action(&player, &fixture.server, Status::FinishedDigging, pos);
    assert_eq!(world.get_block(&pos), &Block::STONE);
    assert!(!player.player.mining.load(Ordering::Relaxed));
    action(&player, &fixture.server, Status::CancelledDigging, pos);
    let next = pos.offset(Vector3::new(1, 0, 0));
    action(&player, &fixture.server, Status::StartedDigging, next);
    player.player.tick_counter.store(200, Ordering::Relaxed);
    player.player.tick_block_breaking();
    assert!(world.get_block_state(&pos).is_air());
    assert_eq!(world.get_block(&next), &Block::STONE);
    action(&player, &fixture.server, Status::FinishedDigging, next);
    assert!(world.get_block_state(&next).is_air());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_life_reset_abandons_active_and_delayed_targets() {
    let (fixture, player, pos) = fixture().await;
    let world = fixture.world();
    for delayed in [false, true] {
        action(&player, &fixture.server, Status::StartedDigging, pos);
        if delayed {
            action(&player, &fixture.server, Status::FinishedDigging, pos);
        }
        // Pumpkin respawn reuses Player and calls this reset instead of constructing a new one.
        player.player.living_entity.reset_state();
        player
            .player
            .get_entity()
            .on_ground
            .store(true, Ordering::Relaxed);
        player.player.tick_counter.fetch_add(200, Ordering::Relaxed);
        if !delayed {
            action(&player, &fixture.server, Status::FinishedDigging, pos);
        }
        player.player.tick_block_breaking();
        assert_eq!(world.get_block(&pos), &Block::STONE, "delayed={delayed}");
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_java_delayed_destroy_clears_active_crack_stage() {
    let (fixture, player, pos) = fixture().await;
    action(&player, &fixture.server, Status::StartedDigging, pos);
    player
        .player
        .current_block_destroy_stage
        .store(4, Ordering::Relaxed);
    action(&player, &fixture.server, Status::FinishedDigging, pos);
    assert!(!player.player.mining.load(Ordering::Relaxed));
    assert_eq!(
        player
            .player
            .current_block_destroy_stage
            .load(Ordering::Relaxed),
        -1
    );
    fixture.server.shutdown().await;
}
