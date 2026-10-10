#![expect(
    clippy::unwrap_used,
    reason = "Respawn regression fixtures must be valid"
)]

use super::cramming;
use crate::{
    command::argument_builder::ArgumentBuilder,
    entity::{EntityBase, player::Player},
    net::{ClientPlatform, java::combat_test_support::TestPlayer},
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        entity::{entity_damage::EntityDamageEvent, entity_toggle_glide::EntityToggleGlideEvent},
        player::{
            player_bed::PlayerBedLeaveEvent, player_command_send::PlayerCommandSendEvent,
            player_input::PlayerInputEvent, player_toggle_sneak_event::PlayerToggleSneakEvent,
            player_toggle_sprint_event::PlayerToggleSprintEvent,
        },
    },
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
use pumpkin_data::{
    Block,
    dimension::Dimension,
    entity::EntityType,
    game_rules::{GameRule, GameRuleValue},
};
use pumpkin_protocol::java::server::play::{Action, SChatCommand, SPlayerCommand, SPlayerInput};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct Complete {
    player: Weak<Player>,
    after: fn(&Player),
    calls: Arc<AtomicUsize>,
}
impl<E: Payload> EventHandler<E> for Complete {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = self.player.upgrade().unwrap();
            let lifecycle = player.living_entity.damage_lifecycle();
            player.world().respawn_player(&player, false).await;
            assert!(!player.living_entity.is_respawning());
            assert_ne!(player.living_entity.damage_lifecycle(), lifecycle);
            (self.after)(&player);
            self.calls.fetch_add(1, Relaxed);
        })
    }
}
pub(super) fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    player: &Arc<Player>,
    after: fn(&Player),
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    server.plugin_manager.register::<E, _>(
        Arc::new(Complete {
            player: Arc::downgrade(player),
            after,
            calls: calls.clone(),
        }),
        EventPriority::Normal,
        true,
    );
    calls
}
pub(super) fn prepare(server: &Server, player: &Arc<Player>) {
    let world = player.world();
    server.worlds.store(Arc::new(vec![world.clone()]));
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(12, 63, 12, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    player.set_respawn_point(
        Dimension::OVERWORLD,
        BlockPos::new(12, 64, 12),
        0.0,
        0.0,
        true,
    );
    player.get_entity().set_pos(Vector3::new(4.5, 100.0, 4.5));
}
async fn run(
    fixture: &mut TestPlayer,
    server: &Arc<Server>,
    action: impl FnOnce(&Arc<Player>, &Arc<Server>) + Send + 'static,
) {
    let player = fixture.player.clone();
    let server = server.clone();
    fixture
        .collect_packets_during(async move {
            tokio::task::spawn_blocking(move || action(&player, &server))
                .await
                .unwrap();
        })
        .await;
}
fn input(player: &Arc<Player>, server: &Arc<Server>) {
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java fixture required")
    };
    client.handle_player_input(
        player,
        &SPlayerInput {
            input: SPlayerInput::SNEAK,
        },
        server,
    );
}
fn command(player: &Arc<Player>, server: &Arc<Server>, action: Action) {
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java fixture required")
    };
    client.handle_player_command(
        player,
        &SPlayerCommand {
            entity_id: player.entity_id().into(),
            action,
            jump_boost: 0.into(),
        },
        server,
    );
}
fn mount(player: &Arc<Player>) -> Arc<dyn EntityBase> {
    let cart = crate::entity::r#type::from_type(
        &EntityType::MINECART,
        player.position(),
        &player.world(),
        uuid::Uuid::new_v4(),
    );
    cart.get_entity()
        .add_passenger(cart.clone(), player.clone());
    cart
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_input_discards_old_input_and_mount_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare(&server, &player);
    let _cart = mount(&player);
    player.last_input.store(SPlayerInput::FORWARD, Relaxed);
    let calls = register::<PlayerInputEvent>(&server, &player, |_| {});
    run(&mut fixture, &server, input).await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(player.last_input.load(Relaxed), SPlayerInput::FORWARD);
    assert!(!player.get_entity().is_sneaking());
    assert!(player.get_entity().has_vehicle());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_sneak_cannot_change_the_restored_life_or_dismount() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare(&server, &player);
    let _cart = mount(&player);
    let calls = register::<PlayerToggleSneakEvent>(&server, &player, |_| {});
    run(&mut fixture, &server, input).await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert!(!player.get_entity().is_sneaking());
    assert!(player.get_entity().has_vehicle());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_sprint_and_glide_commands_discard_old_actions() {
    for action in [
        Action::StartSprinting,
        Action::StopSprinting,
        Action::StartFlyingElytra,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let mut fixture = TestPlayer::new(&world);
        let player = fixture.player.clone();
        prepare(&server, &player);
        let was_sprinting = action == Action::StopSprinting;
        player.set_sprinting(was_sprinting);
        player.get_entity().on_ground.store(false, Relaxed);
        let sprint = register::<PlayerToggleSprintEvent>(&server, &player, |_| {});
        let glide = register::<EntityToggleGlideEvent>(&server, &player, |_| {});
        run(&mut fixture, &server, move |player, server| {
            command(player, server, action);
        })
        .await;
        assert_eq!(sprint.load(Relaxed) + glide.load(Relaxed), 1);
        assert!(!player.living_entity.is_respawning());
        assert!(!player.get_entity().is_fall_flying());
        assert_eq!(player.get_entity().is_sprinting(), was_sprinting);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_chat_command_cannot_execute_on_the_new_life() {
    use crate::command::{
        argument_builder::command,
        context::command_context::CommandContext,
        node::{CommandExecutor, CommandExecutorResult},
    };
    struct Count(Arc<AtomicUsize>);
    impl CommandExecutor for Count {
        fn execute(&self, _context: &CommandContext) -> CommandExecutorResult {
            self.0.fetch_add(1, Relaxed);
            Ok(1)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare(&server, &player);
    let executed = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = (*server.command_dispatcher.load_full()).clone();
    dispatcher.register(command("count", "Counts executions").executes(Count(executed.clone())));
    server.command_dispatcher.store(Arc::new(dispatcher));
    let calls = register::<PlayerCommandSendEvent>(&server, &player, |_| {});
    fixture
        .collect_packets_during(async {
            let ClientPlatform::Java(client) = player.client.as_ref() else {
                panic!("Java fixture required")
            };
            client
                .handle_chat_command(&player, &server, &SChatCommand { command: "count" })
                .await;
        })
        .await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(executed.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_leave_bed_cannot_clear_the_new_sleep_state() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare(&server, &player);
    player
        .sleeping_bed_pos
        .store(Some(BlockPos::new(4, 100, 4)));
    let calls = register::<PlayerBedLeaveEvent>(&server, &player, |player| {
        player
            .sleeping_bed_pos
            .store(Some(BlockPos::new(8, 100, 8)));
    });
    run(&mut fixture, &server, |player, server| {
        command(player, server, Action::LeaveBed);
    })
    .await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(
        player.sleeping_bed_pos.load(),
        Some(BlockPos::new(8, 100, 8))
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_respawn_cramming_stops_before_frozen_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    prepare(&server, &player);
    world.set_game_rule(&GameRule::MaxEntityCramming, GameRuleValue::Int(1));
    for _ in 0..2 {
        world.add_entity_silent(crate::entity::r#type::from_type(
            &EntityType::COW,
            player.position(),
            &world,
            uuid::Uuid::new_v4(),
        ));
    }
    let calls = register::<EntityDamageEvent>(&server, &player, |player| {
        player.get_entity().frozen_ticks.store(17, Relaxed);
    });
    run(&mut fixture, &server, |player, server| {
        cramming::with_damage_roll(|| player.living_entity.tick(player.as_ref(), server));
    })
    .await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(player.get_entity().frozen_ticks.load(Relaxed), 17);
    assert_eq!(player.living_entity.health.load(), 20.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup4_command_dispatch_rechecks_life_after_source_preparation() {
    use crate::{
        command::{
            argument_builder::command,
            context::command_context::CommandContext,
            node::{CommandExecutor, CommandExecutorResult},
        },
        entity::living::damage_transaction::test_hooks::{self, Point},
    };
    struct Count(Arc<AtomicUsize>);
    impl CommandExecutor for Count {
        fn execute(&self, _context: &CommandContext) -> CommandExecutorResult {
            self.0.fetch_add(1, Relaxed);
            Ok(1)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = (*server.command_dispatcher.load_full()).clone();
    dispatcher.register(command("count", "Counts executions").executes(Count(executions.clone())));
    server.command_dispatcher.store(Arc::new(dispatcher));
    let replacement = player.clone();
    let published = Arc::new(AtomicUsize::new(0));
    let calls = published.clone();
    test_hooks::install(move |point| {
        if point == Point::CommandDispatch {
            let source = replacement.living_entity.begin_respawn().unwrap();
            replacement.living_entity.reset_state();
            assert!(replacement.living_entity.publish_respawn(
                &replacement,
                &source,
                &source,
                replacement.position(),
                0.0,
                0.0,
            ));
            calls.fetch_add(1, Relaxed);
        }
    });
    fixture
        .collect_packets_during(async {
            let ClientPlatform::Java(client) = player.client.as_ref() else {
                panic!("Java fixture required")
            };
            client
                .handle_chat_command(&player, &server, &SChatCommand { command: "count" })
                .await;
        })
        .await;
    test_hooks::install(|_| {});
    assert_eq!(published.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(executions.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}
