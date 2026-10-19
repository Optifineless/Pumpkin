#![expect(
    clippy::unwrap_used,
    reason = "These regressions require valid server fixtures"
)]

use super::dolphin_swim_with_player::DolphinSwimWithPlayerGoal;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_potion_effect::EntityPotionEffectEvent,
    },
    server::Server,
    world::World,
};
use pumpkin_data::{
    effect::StatusEffect, entity::EntityType, packet::clientbound::play::UPDATE_MOB_EFFECT,
};
use pumpkin_protocol::{
    codec::var_int::VarInt, java::client::play::CUpdateMobEffect, ser::NetworkReadExt,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

fn add_chunk(world: &Arc<World>) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
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
    assert_eq!(effect.map_or(-1, |effect| effect.duration), duration);
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
