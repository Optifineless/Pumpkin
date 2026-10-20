#![expect(
    clippy::unwrap_used,
    reason = "These regressions require valid server fixtures"
)]

use super::dolphin_swim_with_player::DolphinSwimWithPlayerGoal;
use crate::{
    entity::{
        EntityBase,
        ai::goal::{
            Goal, escape_danger::EscapeDangerGoal, melee_attack::MeleeAttackGoal,
            wander_around::WanderAroundGoal,
        },
        death_test_world::DeathTestWorld,
        player::Player,
    },
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_potion_effect::EntityPotionEffectEvent,
    },
    server::Server,
    world::World,
};
use pumpkin_data::{
    Block, damage::DamageType, effect::StatusEffect, entity::EntityType,
    packet::clientbound::play::UPDATE_MOB_EFFECT, potion::Effect, tag::Taggable,
};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::{
        client::play::CUpdateMobEffect,
        server::play::{Action, SPlayerCommand, SPlayerPosition},
    },
    ser::NetworkReadExt,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed},
};

fn add_chunk(world: &Arc<World>) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
}

fn add_chunk_at(world: &Arc<World>, x: i32, z: i32) {
    let chunk_pos = Vector2::new(x, z);
    if !world.level.loaded_chunks.contains_key(&chunk_pos) {
        world
            .level
            .loaded_chunks
            .insert(chunk_pos, ChunkData::empty_sync(x, z));
    }
}

fn add_water_for_player(world: &Arc<World>, position: Vector3<f64>) {
    let x = position.x.floor() as i32;
    let y = position.y.floor() as i32;
    let z = position.z.floor() as i32;
    add_chunk_at(world, x.div_euclid(16), z.div_euclid(16));
    for water_y in y..=y + 2 {
        world.set_block_state(
            &BlockPos::new(x, water_y, z),
            Block::WATER.default_state.id,
            BlockFlags::FORCE_STATE,
        );
    }
}

fn move_player(fixture: &DeathTestWorld, client: &TestPlayer, position: Vector3<f64>) {
    client.client().handle_position(
        &client.player,
        &fixture.server,
        &SPlayerPosition {
            position,
            collision: 0,
        },
    );
    client.player.tick(&fixture.server);
}

fn start_sprinting_swimmer(fixture: &DeathTestWorld, client: &TestPlayer, position: Vector3<f64>) {
    add_water_for_player(&fixture.world(), position);
    move_player(fixture, client, position);
    client.client().handle_player_command(
        &client.player,
        &SPlayerCommand {
            entity_id: client.player.entity_id().into(),
            action: Action::StartSprinting,
            jump_boost: VarInt(0),
        },
        &fixture.server,
    );
    assert!(client.player.get_entity().is_sprinting());
    assert!(client.player.is_swimming());
}

fn move_sprinting_swimmer(fixture: &DeathTestWorld, client: &TestPlayer, position: Vector3<f64>) {
    add_water_for_player(&fixture.world(), position);
    move_player(fixture, client, position);
    assert!(client.player.is_swimming());
}

fn spawn_dolphin_with_refresh_roll(
    fixture: &DeathTestWorld,
    initial_roll: u32,
) -> (Arc<dyn EntityBase>, Arc<AtomicU32>) {
    let roll = Arc::new(AtomicU32::new(initial_roll));
    let dolphin = DolphinSwimWithPlayerGoal::with_default_refresh_roll(roll.clone(), || {
        spawn_dolphin(fixture)
    });
    (dolphin, roll)
}

fn is_goal_running<G: Goal + 'static>(dolphin: &Arc<dyn EntityBase>) -> bool {
    dolphin
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .goals_selector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_goal_running::<G>()
}

fn add_invisibility(player: &Player) {
    player.living_entity.add_effect(Effect {
        effect_type: &StatusEffect::INVISIBILITY,
        duration: 200,
        amplifier: 0,
        ambient: false,
        show_particles: false,
        show_icon: true,
        blend: false,
    });
}

fn effect_packet_count(packets: &[bytes::Bytes]) -> usize {
    packets
        .iter()
        .filter(|packet| {
            let mut packet = packet.as_ref();
            packet
                .get_var_int()
                .is_ok_and(|id| id.0 == UPDATE_MOB_EFFECT.0)
        })
        .count()
}

fn spawn_dolphin(fixture: &DeathTestWorld) -> Arc<dyn EntityBase> {
    let dolphin = fixture.mob(&EntityType::DOLPHIN);
    dolphin.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    dolphin
}

fn spawn_controlled_dolphin(
    fixture: &DeathTestWorld,
    initial_roll: u32,
) -> (Arc<dyn EntityBase>, Arc<std::sync::atomic::AtomicU32>) {
    let dolphin = spawn_dolphin(fixture);
    let (goal, roll) = DolphinSwimWithPlayerGoal::with_controlled_refresh_roll(initial_roll);
    {
        let mob = dolphin.get_mob().unwrap();
        let mut selector = mob
            .get_mob_entity()
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        selector.remove_goal::<DolphinSwimWithPlayerGoal>(mob);
        // Navigation assertions exclude unrelated random strolling after the follow goal stops.
        selector.remove_goal::<WanderAroundGoal>(mob);
        selector.add_goal(2, Box::new(goal));
    };
    (dolphin, roll)
}

fn tick_dolphin(fixture: &DeathTestWorld, dolphin: &Arc<dyn EntityBase>) {
    dolphin.tick(dolphin.as_ref(), &fixture.server);
}

fn grace_packet(player: &TestPlayer) -> bytes::Bytes {
    player
        .client()
        .serialize_packet(&CUpdateMobEffect::new(
            player.player.get_entity().entity_id.into(),
            VarInt(i32::from(StatusEffect::DOLPHINS_GRACE.id)),
            VarInt(0),
            VarInt(100),
            6,
        ))
        .unwrap()
}

fn has_effect_packet(packets: &[bytes::Bytes]) -> bool {
    packets.iter().any(|packet| {
        let mut packet = packet.as_ref();
        packet
            .get_var_int()
            .is_ok_and(|id| id.0 == UPDATE_MOB_EFFECT.0)
    })
}

fn set_grace_duration(player: &Player, duration: i32) {
    player
        .living_entity
        .active_effects
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_mut(&StatusEffect::DOLPHINS_GRACE)
        .unwrap()
        .duration = duration;
}

fn assert_grace_duration(player: &Player, duration: i32) {
    let effect = player
        .living_entity
        .get_effect(&StatusEffect::DOLPHINS_GRACE);
    assert!(effect.is_some(), "eligible swimmer did not receive Grace");
    assert_eq!(
        effect.as_ref().map_or(-1, |effect| effect.duration),
        duration
    );
    assert_eq!(effect.as_ref().map(|effect| effect.amplifier), Some(0));
}

struct MovePlayerOnGrace {
    player: Arc<Player>,
    destination: Vector3<f64>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<EntityPotionEffectEvent> for MovePlayerOnGrace {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityPotionEffectEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.player.entity_id() {
            self.calls.fetch_add(1, Relaxed);
            let player = self.player.clone();
            let destination = self.destination;
            Box::pin(async move {
                player.get_entity().set_pos(destination);
            })
        } else {
            Box::pin(async {})
        }
    }
}

struct DisconnectPlayerOnGrace {
    world: Arc<World>,
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<EntityPotionEffectEvent> for DisconnectPlayerOnGrace {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityPotionEffectEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.player.entity_id() {
            self.calls.fetch_add(1, Relaxed);
            let world = self.world.clone();
            let player = self.player.clone();
            Box::pin(async move {
                world.remove_player(&player, false).await;
            })
        } else {
            Box::pin(async {})
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_nearby_swimmer_gets_grace_and_dolphin_moves_and_looks() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    client.player.get_entity().set_swimming(true);
    let dolphin = spawn_dolphin(&fixture);
    let expected_packet = grace_packet(&client);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);

    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));
    let mob = dolphin.get_mob().unwrap();
    assert!(
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_in_progress()
    );
    assert_ne!(dolphin.get_entity().head_yaw.load(), 0.0);

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_nonswimmer_out_of_range_and_combat_target_are_rejected() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    let dolphin = spawn_dolphin(&fixture);
    let player = client.player.clone();

    player.get_entity().set_pos(Vector3::new(5.0, 64.0, 0.0));
    player.get_entity().set_swimming(false);
    tick_dolphin(&fixture, &dolphin);
    assert!(
        !player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE)
    );

    player.get_entity().set_swimming(true);
    player.get_entity().set_pos(Vector3::new(11.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert!(
        !player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE)
    );

    player.get_entity().set_pos(Vector3::new(5.0, 64.0, 0.0));
    let combat_target: Arc<dyn EntityBase> = player.clone();
    dolphin
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .set_target(Some(combat_target));
    tick_dolphin(&fixture, &dolphin);
    assert!(
        !player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE)
    );
    assert!(!has_effect_packet(&client.take_packets()));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_goal_stops_and_reacquires_after_player_moves_away() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    client.player.get_entity().set_swimming(true);
    let (dolphin, _) = spawn_controlled_dolphin(&fixture, 5);
    let expected_packet = grace_packet(&client);

    tick_dolphin(&fixture, &dolphin);
    set_grace_duration(&client.player, 1);
    client.take_packets();

    client
        .player
        .get_entity()
        .set_pos(Vector3::new(17.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 1);
    assert!(
        dolphin
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_idle()
    );

    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_refresh_roll_zero_resets_grace_and_other_rolls_do_not() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    client.player.get_entity().set_swimming(true);
    let (dolphin, roll) = spawn_controlled_dolphin(&fixture, 5);
    let expected_packet = grace_packet(&client);

    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    client.take_packets();

    set_grace_duration(&client.player, 1);
    roll.store(1, Relaxed);
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 1);
    assert!(!has_effect_packet(&client.take_packets()));

    roll.store(0, Relaxed);
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_effect_callback_cannot_commit_after_player_moves_away() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    client.player.get_entity().set_swimming(true);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EntityPotionEffectEvent, _>(
            Arc::new(MovePlayerOnGrace {
                player: client.player.clone(),
                destination: Vector3::new(40.0, 64.0, 0.0),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let dolphin = spawn_dolphin(&fixture);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);

    assert_eq!(calls.load(Relaxed), 1);
    assert!(
        !client
            .player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE)
    );
    assert!(!has_effect_packet(&client.take_packets()));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g21_effect_callback_cannot_commit_after_player_disconnects() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.0, 64.0, 0.0));
    client.player.get_entity().set_swimming(true);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EntityPotionEffectEvent, _>(
            Arc::new(DisconnectPlayerOnGrace {
                world: world.clone(),
                player: client.player.clone(),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let dolphin = spawn_dolphin(&fixture);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);

    assert_eq!(calls.load(Relaxed), 1);
    assert!(
        !client
            .player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE)
    );
    assert!(
        !world
            .players
            .load_full()
            .iter()
            .any(|player| Arc::ptr_eq(player, &client.player))
    );
    assert!(!has_effect_packet(&client.take_packets()));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g05_dolphin_visibility_scaled_acquisition() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    start_sprinting_swimmer(&fixture, &client, Vector3::new(5.0, 64.0, 0.0));
    add_invisibility(&client.player);
    assert!(client.player.get_entity().invisible.load(Relaxed));
    let (dolphin, _) = spawn_dolphin_with_refresh_roll(&fixture, 1);
    let expected_packet = grace_packet(&client);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);
    assert!(client.player.is_swimming());
    assert!(
        !client
            .player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE),
        "an invisible swimmer at five blocks must fail scaled acquisition"
    );
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(!has_effect_packet(&client.take_packets()));

    assert!(
        client
            .player
            .living_entity
            .remove_effect(&StatusEffect::INVISIBILITY)
    );
    client.take_packets();
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));

    move_sprinting_swimmer(&fixture, &client, Vector3::new(17.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(
        client
            .player
            .living_entity
            .remove_effect(&StatusEffect::DOLPHINS_GRACE)
    );
    add_invisibility(&client.player);
    move_sprinting_swimmer(&fixture, &client, Vector3::new(1.5, 64.0, 0.0));
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);
    assert!(client.player.get_entity().invisible.load(Relaxed));
    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g05_dolphin_ten_block_start_grants_grace() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    start_sprinting_swimmer(&fixture, &client, Vector3::new(10.0, 64.0, 0.0));
    let (dolphin, roll) = spawn_dolphin_with_refresh_roll(&fixture, 1);
    let expected_packet = grace_packet(&client);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    let initial_packets = client.take_packets();
    assert!(initial_packets.contains(&expected_packet));
    assert_eq!(effect_packet_count(&initial_packets), 1);
    assert!(is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));

    move_sprinting_swimmer(&fixture, &client, Vector3::new(17.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(
        client
            .player
            .living_entity
            .remove_effect(&StatusEffect::DOLPHINS_GRACE)
    );
    client.take_packets();
    move_sprinting_swimmer(&fixture, &client, Vector3::new(10.01, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert!(
        !client
            .player
            .living_entity
            .has_effect(&StatusEffect::DOLPHINS_GRACE),
        "acquisition outside ten blocks must not grant Grace"
    );
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(!has_effect_packet(&client.take_packets()));

    move_sprinting_swimmer(&fixture, &client, Vector3::new(5.0, 64.0, 0.0));
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    client.take_packets();
    roll.store(0, Relaxed);
    move_sprinting_swimmer(&fixture, &client, Vector3::new(15.99, 64.0, 0.0));
    set_grace_duration(&client.player, 1);
    client.take_packets();
    tick_dolphin(&fixture, &dolphin);
    assert_grace_duration(&client.player, 100);
    assert!(has_effect_packet(&client.take_packets()));

    move_sprinting_swimmer(&fixture, &client, Vector3::new(16.0, 64.0, 0.0));
    set_grace_duration(&client.player, 1);
    client.take_packets();
    tick_dolphin(&fixture, &dolphin);
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert_grace_duration(&client.player, 1);
    assert!(!has_effect_packet(&client.take_packets()));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g05_follow_preempts_melee_and_preserves_vanilla_continuation() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    add_chunk_at(&world, 1, 0);
    let mut attacker = TestPlayer::new(&world);
    let mut swimmer = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        attacker.player.clone(),
        swimmer.player.clone(),
    ]));
    start_sprinting_swimmer(&fixture, &swimmer, Vector3::new(20.0, 64.0, 0.0));
    move_player(&fixture, &attacker, Vector3::new(5.0, 64.0, 0.0));
    let (dolphin, _) = spawn_dolphin_with_refresh_roll(&fixture, 1);
    // Block DolphinJumpGoal's step-zero clearance check without changing the production selector.
    world.set_block_state(
        &dolphin.get_entity().block_pos.load().up(),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.level_time.lock().unwrap().world_age = 20;
    let target: Arc<dyn EntityBase> = attacker.player.clone();
    dolphin
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .set_target(Some(target));
    {
        let mob = dolphin.get_mob().unwrap();
        let mut selector = mob
            .get_mob_entity()
            .goals_selector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // RandomSwimmingGoal can randomly claim MOVE before this priority test runs.
        selector.remove_goal::<WanderAroundGoal>(mob);
    };
    assert_eq!(world.get_world_age(), 20);
    assert!(!attacker.player.is_swimming());
    assert!(attacker.player.living_entity.is_alive());
    assert!(
        dolphin
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .get_target()
            .is_some()
    );

    for _ in 0..4 {
        tick_dolphin(&fixture, &dolphin);
        if is_goal_running::<MeleeAttackGoal>(&dolphin) {
            break;
        }
    }
    assert!(is_goal_running::<MeleeAttackGoal>(&dolphin));
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));

    move_sprinting_swimmer(&fixture, &swimmer, Vector3::new(3.0, 64.0, 0.0));
    let expected_packet = grace_packet(&swimmer);
    attacker.take_packets();
    swimmer.take_packets();
    tick_dolphin(&fixture, &dolphin);
    assert!(
        is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin),
        "priority-2 following must preempt an already-running melee goal"
    );
    assert!(!is_goal_running::<MeleeAttackGoal>(&dolphin));
    assert_grace_duration(&swimmer.player, 100);
    assert!(swimmer.take_packets().contains(&expected_packet));

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g05_vanilla_continuation_and_panic_controls() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let mut client = TestPlayer::new(&world);
    start_sprinting_swimmer(&fixture, &client, Vector3::new(5.0, 64.0, 0.0));
    let (dolphin, roll) = spawn_dolphin_with_refresh_roll(&fixture, 1);
    world.level_time.lock().unwrap().world_age = 20;
    let expected_packet = grace_packet(&client);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);
    assert!(is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(!is_goal_running::<MeleeAttackGoal>(&dolphin));
    assert!(
        dolphin
            .get_living_entity()
            .is_some_and(|living| living.get_last_damage_type().is_none()),
        "no panic-causing hit should be present during the continuation control"
    );

    let target: Arc<dyn EntityBase> = client.player.clone();
    dolphin
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .set_target(Some(target));
    roll.store(0, Relaxed);
    set_grace_duration(&client.player, 1);
    client.take_packets();
    tick_dolphin(&fixture, &dolphin);
    assert!(is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert!(!is_goal_running::<MeleeAttackGoal>(&dolphin));
    assert_grace_duration(&client.player, 100);
    assert!(client.take_packets().contains(&expected_packet));

    let water = Vector3::new(1.0, 64.0, 0.0);
    add_water_for_player(&world, water);
    assert!(dolphin.get_living_entity().unwrap().damage(
        client.player.as_ref(),
        1.0,
        DamageType::IN_FIRE
    ));
    dolphin.set_on_fire_for(1.0);
    assert!(dolphin.get_entity().fire_ticks.load(Relaxed) > 0);
    assert!(
        dolphin
            .get_living_entity()
            .and_then(crate::entity::living::LivingEntity::get_last_damage_type)
            .is_some_and(
                |damage| damage.has_tag(&pumpkin_data::tag::DamageType::MINECRAFT_PANIC_CAUSES)
            )
    );
    set_grace_duration(&client.player, 1);
    client.take_packets();

    tick_dolphin(&fixture, &dolphin);
    assert!(is_goal_running::<EscapeDangerGoal>(&dolphin));
    assert!(!is_goal_running::<DolphinSwimWithPlayerGoal>(&dolphin));
    assert_grace_duration(&client.player, 1);
    assert!(!has_effect_packet(&client.take_packets()));

    fixture.server.shutdown().await;
}
