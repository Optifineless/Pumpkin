#![expect(
    clippy::unwrap_used,
    reason = "Lifecycle regression fixtures must be valid"
)]

use super::cramming;
use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        entity::{entity_damage::EntityDamageEvent, entity_toggle_glide::EntityToggleGlideEvent},
        player::{
            player_command_send::PlayerCommandSendEvent, player_input::PlayerInputEvent,
            player_toggle_sneak_event::PlayerToggleSneakEvent,
            player_toggle_sprint_event::PlayerToggleSprintEvent,
        },
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_data::{
    damage::DamageType, entity::EntityType, packet::clientbound::play::SET_ENTITY_MOTION,
};
use pumpkin_protocol::{
    codec::{lp_vector_3d::LpVector3d, var_int::VarInt},
    java::server::play::{Action, SChatCommand, SPlayerCommand, SPlayerInput},
    ser::NetworkReadExt,
};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct Handler<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Handler<F> {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&mut E) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<E, _>(Arc::new(Handler(action)), EventPriority::Normal, true);
}
fn restore(player: &Player) {
    assert!(player.living_entity.begin_respawn().is_some());
    player.living_entity.reset_state();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_airborne_rider_dismount_damage_has_no_accumulated_fall_velocity() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player.get_entity().set_pos(Vector3::new(4.5, 100.0, 4.5));
    let cart = crate::entity::r#type::from_type(
        &EntityType::MINECART,
        player.position(),
        &world,
        uuid::Uuid::new_v4(),
    );
    cart.get_entity()
        .add_passenger(cart.clone(), player.clone());
    player
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, -0.6, 0.0));
    for _ in 0..25 {
        player.living_entity.fall_distance.store(12.0);
        player.get_entity().on_ground.store(false, Relaxed);
        player.living_entity.tick(player.as_ref(), &server);
        assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    }
    assert_eq!(player.get_entity().velocity.load(), Vector3::default());
    cart.get_entity().remove_passenger_sync(player.entity_id());
    assert!(!player.get_entity().has_vehicle());
    player.get_entity().on_ground.store(false, Relaxed);
    let attacker = crate::entity::r#type::from_type(
        &EntityType::ZOMBIE,
        player.position() + Vector3::new(0.0, 0.0, 1.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    fixture.take_packets();
    assert!(player.damage_with_context(
        player.as_ref(),
        1.0,
        DamageType::MOB_ATTACK,
        None,
        Some(attacker.as_ref()),
        Some(attacker.as_ref())
    ));
    player.living_entity.flush_tracked_player_motion();
    let motions: Vec<_> = fixture
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut bytes = packet.as_ref();
            if bytes.get_var_int().unwrap().0 != SET_ENTITY_MOTION.0 {
                return None;
            }
            assert_eq!(bytes.get_var_int().unwrap().0, player.entity_id());
            Some(LpVector3d::read(&mut bytes).unwrap().0)
        })
        .collect();
    assert_eq!(motions.len(), 1);
    assert_eq!(motions[0].y, 0.0);
    assert_eq!(player.get_entity().velocity.load().y, 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_cramming_respawn_callback_stops_before_frozen_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    world.set_game_rule(&GameRule::MaxEntityCramming, GameRuleValue::Int(1));
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player.get_entity().set_pos(Vector3::new(4.5, 100.0, 4.5));
    for _ in 0..2 {
        let cow = crate::entity::r#type::from_type(
            &EntityType::COW,
            player.position(),
            &world,
            uuid::Uuid::new_v4(),
        );
        world.add_entity_silent(cow);
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let weak = Arc::downgrade(player);
    register::<EntityDamageEvent>(&server, move |event| {
        assert_eq!(event.damage_type, DamageType::CRAMMING);
        let player = weak.upgrade().unwrap();
        restore(&player);
        player.get_entity().frozen_ticks.store(17, Relaxed);
        seen.fetch_add(1, Relaxed);
    });
    cramming::with_damage_roll(|| player.living_entity.tick(player.as_ref(), &server));
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(player.get_entity().frozen_ticks.load(Relaxed), 17);
    assert_eq!(player.living_entity.health.load(), 20.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_input_respawn_callback_discards_old_input_and_mount_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let cart = crate::entity::r#type::from_type(
        &EntityType::MINECART,
        player.position(),
        &world,
        uuid::Uuid::new_v4(),
    );
    cart.get_entity()
        .add_passenger(cart.clone(), player.clone());
    player.last_input.store(SPlayerInput::FORWARD, Relaxed);
    register::<PlayerInputEvent>(&server, |event| restore(&event.player));
    fixture.client().handle_player_input(
        player,
        &SPlayerInput {
            input: SPlayerInput::SNEAK,
        },
        &server,
    );
    assert!(player.living_entity.is_respawning());
    assert_eq!(player.last_input.load(Relaxed), SPlayerInput::FORWARD);
    assert!(!player.get_entity().is_sneaking());
    assert!(player.get_entity().has_vehicle());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_sneak_callback_cannot_change_the_restored_life_or_dismount() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    let cart = crate::entity::r#type::from_type(
        &EntityType::MINECART,
        player.position(),
        &world,
        uuid::Uuid::new_v4(),
    );
    cart.get_entity()
        .add_passenger(cart.clone(), player.clone());
    register::<PlayerToggleSneakEvent>(&server, |event| restore(&event.player));
    fixture.client().handle_player_input(
        player,
        &SPlayerInput {
            input: SPlayerInput::SNEAK,
        },
        &server,
    );
    assert!(player.living_entity.is_respawning());
    assert!(!player.get_entity().is_sneaking());
    assert!(player.get_entity().has_vehicle());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_sprint_and_glide_command_callbacks_discard_old_actions() {
    for action in [
        Action::StartSprinting,
        Action::StopSprinting,
        Action::StartFlyingElytra,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let was_sprinting = action == Action::StopSprinting;
        player.set_sprinting(was_sprinting);
        player.get_entity().on_ground.store(false, Relaxed);
        register::<PlayerToggleSprintEvent>(&server, |event| restore(&event.player));
        let weak = Arc::downgrade(player);
        register::<EntityToggleGlideEvent>(&server, move |_| restore(&weak.upgrade().unwrap()));
        fixture.client().handle_player_command(
            player,
            &SPlayerCommand {
                entity_id: player.entity_id().into(),
                action,
                jump_boost: VarInt(0),
            },
            &server,
        );
        assert!(player.living_entity.is_respawning());
        assert!(!player.get_entity().is_fall_flying());
        // reset_state leaves sprinting unchanged, so stale stop/start must not change it.
        assert_eq!(player.get_entity().is_sprinting(), was_sprinting);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_chat_command_callback_cannot_execute_on_the_new_life() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    server.worlds.store(Arc::new(vec![world]));
    fixture
        .client()
        .handle_chat_command(&player, &server, &SChatCommand { command: "list" })
        .await;
    assert!(
        fixture
            .take_packets()
            .iter()
            .any(|packet| packet.as_ref().get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0)
    );
    register::<PlayerCommandSendEvent>(&server, |event| restore(&event.player));
    fixture
        .client()
        .handle_chat_command(&player, &server, &SChatCommand { command: "list" })
        .await;
    assert!(player.living_entity.is_respawning());
    assert!(
        !fixture
            .take_packets()
            .iter()
            .any(|packet| packet.as_ref().get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_review_leave_bed_callback_cannot_clear_the_new_sleep_state() {
    use crate::plugin::player::player_bed::PlayerBedLeaveEvent;
    use pumpkin_util::math::position::BlockPos;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), ChunkData::empty_sync(0, 0));
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player
        .sleeping_bed_pos
        .store(Some(BlockPos::new(4, 100, 4)));
    register::<PlayerBedLeaveEvent>(&server, |event| {
        restore(&event.player);
        event
            .player
            .sleeping_bed_pos
            .store(Some(BlockPos::new(8, 100, 8)));
    });
    fixture.client().handle_player_command(
        player,
        &SPlayerCommand {
            entity_id: player.entity_id().into(),
            action: Action::LeaveBed,
            jump_boost: VarInt(0),
        },
        &server,
    );
    assert!(player.living_entity.is_respawning());
    assert_eq!(
        player.sleeping_bed_pos.load(),
        Some(BlockPos::new(8, 100, 8))
    );
    crate::server::fixture_lifecycle::finish().await;
}
