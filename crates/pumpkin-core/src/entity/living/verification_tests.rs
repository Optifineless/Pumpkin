#![expect(
    clippy::unwrap_used,
    reason = "Command regression fixtures must be valid"
)]

use crate::{
    command::{
        argument_builder::{ArgumentBuilder, command},
        context::command_context::CommandContext,
        node::{CommandExecutor, CommandExecutorResult},
    },
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    plugin::player::player_command_send::PlayerCommandSendEvent,
    server::combat_test_support::{server, world},
};
use pumpkin_protocol::{
    bedrock::server::command_request::{CommandOriginData, SCommandRequest},
    java::server::play::SChatCommand,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

struct KillSender(Arc<AtomicBool>);
impl CommandExecutor for KillSender {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let sender = context.source.entity_or_err()?;
        let runtime = tokio::runtime::Handle::current();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _runtime = runtime.enter();
            sender.kill(sender.as_ref());
            let _ = done.send(());
        });
        let completed = finished
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        self.0.store(completed, Relaxed);
        if completed {
            worker.join().unwrap();
        }
        Ok(1)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_native_suicide_command_can_kill_its_sender_from_another_thread() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let completed = Arc::new(AtomicBool::new(false));
    let mut dispatcher = (*server.command_dispatcher.load_full()).clone();
    dispatcher
        .register(command("suicide", "Kills the sender").executes(KillSender(completed.clone())));
    server.command_dispatcher.store(Arc::new(dispatcher));
    fixture
        .client()
        .handle_chat_command(
            &fixture.player,
            &server,
            &SChatCommand { command: "suicide" },
        )
        .await;
    assert!(
        completed.load(Relaxed),
        "command kept sender combat ownership while waiting for kill"
    );
    assert_eq!(fixture.player.living_entity.health.load(), 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

struct Count(Arc<AtomicUsize>);
impl CommandExecutor for Count {
    fn execute(&self, _context: &CommandContext) -> CommandExecutorResult {
        self.0.fetch_add(1, Relaxed);
        Ok(1)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_bedrock_command_discards_a_completed_respawn_callback() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestBedrockPlayer::new(&world).await;
    let player = fixture.player.clone();
    super::completed_respawn_tests::prepare(&server, &player);
    let executed = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = (*server.command_dispatcher.load_full()).clone();
    dispatcher.register(command("count", "Counts executions").executes(Count(executed.clone())));
    server.command_dispatcher.store(Arc::new(dispatcher));
    let calls = super::completed_respawn_tests::register::<PlayerCommandSendEvent>(
        &server,
        &player,
        |_| {},
    );
    fixture
        .collect_packets_during(async {
            let crate::net::ClientPlatform::Bedrock(client) = player.client.as_ref() else {
                panic!("Bedrock fixture required")
            };
            client
                .handle_chat_command(
                    &player,
                    &server,
                    SCommandRequest {
                        command: "/count".into(),
                        origin: CommandOriginData {
                            r#type: "player".into(),
                            uuid: player.gameprofile.id,
                            request_id: "test".into(),
                            player_id: i64::from(player.entity_id()),
                        },
                        is_internal: false,
                        version: "".into(),
                    },
                )
                .await;
        })
        .await;
    assert_eq!(calls.load(Relaxed), 1);
    assert!(!player.living_entity.is_respawning());
    assert_eq!(executed.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_living_reset_leaves_player_hunger_to_the_respawn_caller() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    fixture.player.hunger_manager.set_level(7);
    fixture.player.hunger_manager.set_saturation(2.0);
    fixture.player.living_entity.reset_state();
    assert_eq!(fixture.player.hunger_manager.level.load(), 7);
    assert_eq!(fixture.player.hunger_manager.saturation.load(), 2.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "is_owned_by_current_thread")]
async fn verification_player_tick_life_requires_ownership_when_checked() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let life = {
        let _owner = fixture.player.living_entity.own_damage();
        super::PlayerTickLife::capture(fixture.player.as_ref())
    };
    life.is_current();
    crate::server::fixture_lifecycle::finish().await;
}
