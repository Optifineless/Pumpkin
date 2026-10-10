use crate::command::{CommandSender, CommandSource};
use crate::entity::EntityBase;
use crate::net::java::combat_test_support::TestPlayer;
use crate::server::{Server, combat_test_support};
use crate::world::World;
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_protocol::java::server::play::SChatCommand;
use pumpkin_util::{GameMode, PermissionLvl, math::vector3::Vector3};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn packet_id(mut packet: &[u8]) -> i32 {
    pumpkin_protocol::codec::var_int::VarInt::decode(&mut packet)
        .unwrap()
        .0
}

struct Fixture {
    directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    alice: TestPlayer,
    bob: TestPlayer,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let alice = TestPlayer::new(&world);
        let bob = TestPlayer::new(&world);
        world
            .players
            .store(Arc::new(vec![alice.player.clone(), bob.player.clone()]));
        server.worlds.store(Arc::new(vec![world.clone()]));
        alice.player.permission_lvl.store(PermissionLvl::Four);
        alice
            .player
            .get_entity()
            .add_scoreboard_tag("command_alice");
        bob.player.get_entity().add_scoreboard_tag("command_bob");
        Ok(Self {
            directory,
            server,
            world,
            alice,
            bob,
        })
    }

    fn sources(&self) -> [CommandSource; 2] {
        [
            self.alice.player.get_command_source(&self.server),
            CommandSender::Console.into_source(&self.server),
        ]
    }

    fn run_as_bob(&self, source: &CommandSource, command: &str) -> Result<i32, String> {
        self.server
            .command_dispatcher
            .load()
            .execute_input(
                &format!("execute as @a[tag=command_bob,limit=1] run {command}"),
                source,
            )
            .map_err(|error| format!("{error:?}"))
    }
}

#[tokio::test]
async fn malformed_unicode_msg_reports_error_without_panicking() -> TestResult {
    let fixture = Fixture::new()?;
    fixture
        .alice
        .player
        .permission_lvl
        .store(PermissionLvl::Zero);
    let command = "msg \"😀😀😀";
    let error = fixture
        .server
        .command_dispatcher
        .load()
        .execute_input(command, &fixture.sources()[0])
        .expect_err("unterminated quote");
    assert_eq!(error.context.expect("quote error context").cursor, 17);
    fixture
        .alice
        .client()
        .handle_chat_command(
            &fixture.alice.player,
            &fixture.server,
            &SChatCommand { command },
        )
        .await;
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_clear_targets_executing_entity() -> TestResult {
    let fixture = Fixture::new()?;
    for source in fixture.sources() {
        for command in ["clear @s", "clear"] {
            fixture
                .alice
                .player
                .inventory
                .set_held_item(ItemStack::new(64, &Item::DIAMOND));
            fixture
                .bob
                .player
                .inventory
                .set_held_item(ItemStack::new(6, &Item::GOLD_INGOT));
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            assert_eq!(fixture.alice.player.inventory.held_item().item_count, 64);
            assert!(fixture.bob.player.inventory.held_item().is_empty());
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_gamemode_matches_self_selector() -> TestResult {
    let mut fixture = Fixture::new()?;
    for source in fixture.sources() {
        for command in ["gamemode creative @s", "gamemode creative"] {
            fixture.bob.player.set_gamemode(GameMode::Survival);
            fixture.alice.take_packets();
            fixture.bob.take_packets();
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            assert_eq!(fixture.alice.player.gamemode.load(), GameMode::Survival);
            assert_eq!(fixture.bob.player.gamemode.load(), GameMode::Creative);
            let chat = pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0;
            assert!(
                fixture
                    .alice
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == chat)
            );
            assert!(
                !fixture
                    .bob
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == chat)
            );
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_spawnpoint_matches_self_selector() -> TestResult {
    let fixture = Fixture::new()?;
    for source in fixture.sources() {
        for command in ["spawnpoint @s", "spawnpoint"] {
            *fixture.bob.player.respawn_point.lock().unwrap() = None;
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            assert!(fixture.alice.player.respawn_point.lock().unwrap().is_none());
            let respawn = fixture
                .bob
                .player
                .respawn_point
                .lock()
                .unwrap()
                .clone()
                .unwrap();
            assert_eq!(
                respawn.position,
                pumpkin_util::math::position::BlockPos::floored_v(source.position)
            );
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_transfer_matches_self_selector() -> TestResult {
    let mut fixture = Fixture::new()?;
    for source in fixture.sources() {
        for command in [
            "transfer example.invalid 25565 @s",
            "transfer example.invalid 25565",
            "transfer example.invalid",
        ] {
            fixture.alice.take_packets();
            fixture.bob.take_packets();
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            // Feedback may reach Alice; the transfer packet must reach only Bob.
            let transfer = pumpkin_data::packet::clientbound::play::TRANSFER.0;
            assert!(
                !fixture
                    .alice
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == transfer)
            );
            assert!(
                fixture
                    .bob
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == transfer)
            );
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_spectate_matches_self_selector() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.bob.player.set_gamemode(GameMode::Spectator);
    let target = "@a[tag=command_alice,limit=1]";
    for source in fixture.sources() {
        for command in [
            format!("spectate {target} @s"),
            format!("spectate {target}"),
        ] {
            fixture.bob.player.camera_target_id.store(None);
            assert_eq!(fixture.run_as_bob(&source, &command)?, 1);
            assert_eq!(
                fixture.bob.player.camera_target_id.load(),
                Some(fixture.alice.player.entity_id())
            );
            assert_eq!(fixture.alice.player.camera_target_id.load(), None);
            assert_eq!(fixture.run_as_bob(&source, "spectate")?, 1);
            assert_eq!(fixture.bob.player.camera_target_id.load(), None);
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_implicit_playsound_matches_self_selector() -> TestResult {
    let mut fixture = Fixture::new()?;
    fixture
        .alice
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    fixture
        .bob
        .player
        .get_entity()
        .set_pos(Vector3::new(1.0, 64.0, 0.0));
    for source in fixture
        .sources()
        .map(|source| source.with_position(Vector3::new(0.0, 64.0, 0.0)))
    {
        for command in [
            "playsound minecraft:entity.cat.ambient master @s",
            "playsound minecraft:entity.cat.ambient master",
            "playsound minecraft:entity.cat.ambient",
        ] {
            fixture.alice.take_packets();
            fixture.bob.take_packets();
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            let sound = pumpkin_data::packet::clientbound::play::SOUND.0;
            assert!(
                !fixture
                    .alice
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == sound)
            );
            assert!(
                fixture
                    .bob
                    .take_packets()
                    .iter()
                    .any(|packet| packet_id(packet) == sound)
            );
        }
    }
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_run_respects_disabled_command_configuration() -> TestResult {
    let fixture = Fixture::new()?;
    let source = &fixture.sources()[0];
    let mut dispatcher = (*fixture.server.command_dispatcher.load_full()).clone();
    dispatcher.disable_command("gamemode");
    for input in [
        "gamemode creative",
        "execute run gamemode creative",
        "execute run execute as @s run execute run gamemode creative",
        "return run gamemode creative",
    ] {
        assert!(dispatcher.execute_input(input, source).is_err(), "{input}");
        assert_eq!(fixture.alice.player.gamemode.load(), GameMode::Survival);
    }
    let suggestions = dispatcher.suggest("execute run game", source);
    assert!(
        suggestions
            .iter()
            .any(|entry| entry.suggestion == "gamerule")
    );
    assert!(
        suggestions
            .iter()
            .all(|entry| entry.suggestion != "gamemode")
    );
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[tokio::test]
async fn execute_as_raid_uses_executing_player_and_source_position() -> TestResult {
    let fixture = Fixture::new()?;
    fixture
        .alice
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    fixture
        .bob
        .player
        .get_entity()
        .set_pos(Vector3::new(1000.0, 64.0, 0.0));
    let alice_pos = fixture.alice.player.get_entity().block_pos.load();
    let bob_pos = fixture.bob.player.get_entity().block_pos.load();
    {
        let mut raids = fixture.world.raids.lock().unwrap();
        raids.create_or_extend_raid(alice_pos, &fixture.world);
        raids.create_or_extend_raid(bob_pos, &fixture.world)
    };
    for source in fixture.sources() {
        for command in ["execute as @s run raid setomen 3", "raid setomen 3"] {
            fixture
                .world
                .raids
                .lock()
                .unwrap()
                .get_raid_at_mut(&bob_pos)
                .unwrap()
                .set_raid_omen_level(1);
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
            let raids = fixture.world.raids.lock().unwrap();
            assert_eq!(
                raids.get_raid_at(&bob_pos).unwrap().get_raid_omen_level(),
                3
            );
            assert_ne!(
                raids.get_raid_at(&alice_pos).unwrap().get_raid_omen_level(),
                3
            );
        }
        for command in ["raid check", "raid glow", "raid sound"] {
            assert_eq!(fixture.run_as_bob(&source, command)?, 1);
        }
        assert_eq!(fixture.run_as_bob(&source, "raid stop")?, 1);
        assert_eq!(fixture.run_as_bob(&source, "raid start 2")?, 1);
        assert_eq!(
            fixture
                .world
                .raids
                .lock()
                .unwrap()
                .get_raid_at(&bob_pos)
                .unwrap()
                .get_raid_omen_level(),
            2
        );
    }
    let source = CommandSender::Console
        .into_source(&fixture.server)
        .with_position(Vector3::new(12.0, 64.0, 34.0));
    assert_eq!(fixture.run_as_bob(&source, "raid spawnleader")?, 1);
    let entities = fixture.world.entities.load();
    assert_eq!(
        entities.last().unwrap().get_entity().pos.load(),
        source.position
    );
    crate::server::fixture_lifecycle::finish().await;
    Ok(())
}

#[path = "review_followup_tests.rs"]
mod followup;

#[path = "verification_tests.rs"]
mod verification;
