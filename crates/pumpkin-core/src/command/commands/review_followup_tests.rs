use super::*;
use crate::command::argument_builder::{ArgumentBuilder, command};
use crate::command::context::command_context::CommandContext;
use crate::command::node::CommandExecutorResult;
use pumpkin_command::source::CommandSource as _;
use pumpkin_util::math::vector2::Vector2;
use std::sync::Mutex;

struct Inspect(Arc<Mutex<Vec<CommandSource>>>);

impl crate::command::node::CommandExecutor for Inspect {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        self.0.lock().unwrap().push(context.source.as_ref().clone());
        Ok(7)
    }
}

fn inspect_dispatcher(
    fixture: &Fixture,
) -> (
    crate::command::CommandDispatcher,
    Arc<Mutex<Vec<CommandSource>>>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = (*fixture.server.command_dispatcher.load_full()).clone();
    dispatcher.register(command("inspect", "capture context").executes(Inspect(seen.clone())));
    (dispatcher, seen)
}

fn waypoint_packets(player: &mut TestPlayer) -> Vec<bytes::Bytes> {
    player
        .take_packets()
        .into_iter()
        .filter(|packet| packet_id(packet) == pumpkin_data::packet::clientbound::play::WAYPOINT.0)
        .collect()
}

fn waypoint_in_range(fixture: &Fixture) -> Arc<dyn EntityBase> {
    let mob = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
        Vector3::new(1.0, 64.0, 2.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    fixture.world.entities.store(Arc::new(vec![mob.clone()]));
    mob.get_living_entity().unwrap().set_attribute_base(
        &pumpkin_data::attributes::Attributes::WAYPOINT_TRANSMIT_RANGE,
        100.0,
    );
    for player in [&fixture.alice.player, &fixture.bob.player] {
        player.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
        player.living_entity.set_attribute_base(
            &pumpkin_data::attributes::Attributes::WAYPOINT_RECEIVE_RANGE,
            100.0,
        );
    }
    mob
}

fn assert_waypoint_packet(packet: &[u8], operation: u8, uuid: uuid::Uuid, suffix: &[u8]) {
    let mut data = packet;
    pumpkin_protocol::codec::var_int::VarInt::decode(&mut data).unwrap();
    // TrackedWaypoint.write: operation, UUID discriminator and UUID, followed by icon and target.
    let mut expected = vec![operation, 1];
    expected.extend_from_slice(uuid.as_bytes());
    expected.extend_from_slice(suffix);
    assert_eq!(data, expected);
}

#[tokio::test]
async fn local_coordinates_use_context_position_and_numeric_positioned_resets_anchor() -> TestResult
{
    let fixture = Fixture::new()?;
    fixture
        .bob
        .player
        .get_entity()
        .set_pos(Vector3::new(1.0, 2.0, 3.0));
    let source = fixture.sources()[0]
        .clone()
        .with_entity(fixture.bob.player.clone())
        .with_position(Vector3::new(100.0, 80.0, 100.0));
    let feet = crate::command::argument_types::entity_anchor::EntityAnchor::Feet;
    let eyes = crate::command::argument_types::entity_anchor::EntityAnchor::Eyes;
    assert_eq!(
        source.anchor_position(feet),
        Vector3::new(100.0, 80.0, 100.0)
    );
    assert_eq!(
        source.anchor_position(eyes),
        Vector3::new(
            100.0,
            80.0 + fixture.bob.player.get_entity().get_eye_height(),
            100.0
        )
    );
    let (dispatcher, seen) = inspect_dispatcher(&fixture);
    dispatcher
        .execute_input(
            "execute anchored eyes positioned 100 80 100 positioned ^ ^ ^ run inspect",
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    let seen = seen.lock().unwrap();
    // Vec3Argument centers integer X/Z before numeric positioned resets the anchor.
    assert_eq!(seen[0].position, Vector3::new(100.5, 80.0, 100.5));
    assert_eq!(seen[0].entity_anchor, feet);
    Ok(())
}

fn dimension_world(fixture: &Fixture, dimension: pumpkin_data::dimension::Dimension) -> Arc<World> {
    Arc::new(World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            fixture.directory.path().to_path_buf(),
            0,
            dimension.clone(),
        ),
        fixture.server.level_info.clone(),
        dimension,
        fixture.server.block_registry.clone(),
        Arc::downgrade(&fixture.server),
    ))
}

#[tokio::test]
async fn execute_in_scales_both_directions_and_preserves_source_state() -> TestResult {
    let fixture = Fixture::new()?;
    let nether = dimension_world(&fixture, pumpkin_data::dimension::Dimension::THE_NETHER);
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone(), nether.clone()]));
    let (dispatcher, seen) = inspect_dispatcher(&fixture);
    let mut source = fixture.sources()[0]
        .clone()
        .with_position(Vector3::new(800.0, 80.0, -800.0));
    source.silent = true;
    for input in [
        "execute in minecraft:the_nether run inspect",
        "execute in minecraft:the_nether in minecraft:overworld run inspect",
    ] {
        dispatcher
            .execute_input(input, &source)
            .map_err(|error| format!("{error:?}"))?;
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].position, Vector3::new(100.0, 80.0, -100.0));
    assert_eq!(seen[1].position, source.position);
    assert!(Arc::ptr_eq(seen[0].world(), &nether));
    for captured in seen.iter() {
        assert!(Arc::ptr_eq(
            captured.entity.as_ref().unwrap(),
            source.entity.as_ref().unwrap()
        ));
        assert!(captured.silent);
        assert!(captured.has_permission("minecraft:command.gamemode"));
        assert!(Arc::ptr_eq(
            &captured.as_player().unwrap(),
            &fixture.alice.player
        ));
    }
    Ok(())
}

#[tokio::test]
async fn execute_rotation_modifiers_use_pitch_then_yaw_and_anchored_facing() -> TestResult {
    let fixture = Fixture::new()?;
    let entity = fixture.bob.player.get_entity();
    entity.pitch.store(30.0);
    entity.yaw.store(60.0);
    entity.set_pos(Vector3::new(10.0, 20.0, 30.0));
    let (dispatcher, seen) = inspect_dispatcher(&fixture);
    let source = fixture.sources()[0]
        .clone()
        .with_position(Vector3::new(0.0, 0.0, 0.0));
    for input in [
        "execute at @a[tag=command_bob,limit=1] run inspect",
        "execute rotated as @a[tag=command_bob,limit=1] run inspect",
        "execute rotated 60 30 run inspect",
        "execute facing -3.0 -2.0 1.0 run inspect",
        "execute anchored eyes facing entity @a[tag=command_bob,limit=1] eyes run inspect",
    ] {
        dispatcher
            .execute_input(input, &source)
            .map_err(|error| format!("{error:?}"))?;
    }
    let seen = seen.lock().unwrap();
    for captured in &seen[..3] {
        assert_eq!(captured.rotation, Vector2::new(30.0, 60.0));
    }
    assert!((seen[3].rotation.x - 32.311535).abs() < 0.0001);
    assert!((seen[3].rotation.y - 71.56505).abs() < 0.0001);
    assert!((seen[4].rotation.x + 32.311535).abs() < 0.0001);
    assert!((seen[4].rotation.y + 18.434948).abs() < 0.0001);
    Ok(())
}

#[tokio::test]
async fn top_level_return_run_executes_only_first_of_two_players() -> TestResult {
    let fixture = Fixture::new()?;
    let (dispatcher, seen) = inspect_dispatcher(&fixture);
    let source = fixture.sources()[1].clone();
    dispatcher
        .execute_input("execute as @a run inspect", &source)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(seen.lock().unwrap().len(), 2);
    seen.lock().unwrap().clear();
    dispatcher
        .execute_input("return run execute as @a run inspect", &source)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(dispatcher.execute_input("return 4", &source), Ok(4));
    Ok(())
}

#[tokio::test]
async fn entity_uuid_selectors_resolve_players_and_mobs_without_relaxing_player_arguments()
-> TestResult {
    let fixture = Fixture::new()?;
    let mob = crate::entity::r#type::from_type(
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
        Vector3::new(1.0, 64.0, 2.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    fixture.world.entities.store(Arc::new(vec![mob.clone()]));
    let (dispatcher, seen) = inspect_dispatcher(&fixture);
    for uuid in [
        fixture.bob.player.get_entity().entity_uuid,
        mob.get_entity().entity_uuid,
    ] {
        dispatcher
            .execute_input(
                &format!("execute as {uuid} run inspect"),
                &fixture.sources()[1],
            )
            .map_err(|error| format!("{error:?}"))?;
        assert!(
            dispatcher
                .execute_input(&format!("msg {uuid} hello"), &fixture.sources()[1])
                .is_err()
        );
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0].entity.as_ref().unwrap().get_entity().entity_uuid,
        fixture.bob.player.get_entity().entity_uuid
    );
    assert_eq!(
        seen[1].entity.as_ref().unwrap().get_entity().entity_uuid,
        mob.get_entity().entity_uuid
    );
    Ok(())
}

#[tokio::test]
async fn waypoint_console_persists_icons_without_tracking_unmanaged_viewers() -> TestResult {
    let mut fixture = Fixture::new()?;
    let mob = waypoint_in_range(&fixture);
    let living = mob.get_living_entity().unwrap();
    let source = fixture.sources()[1].clone();
    let dispatcher = fixture.server.command_dispatcher.load();
    for (suffix, color, style) in [
        ("color hex 123456", Some(0x123456), None),
        (
            "style set minecraft:bowtie",
            Some(0x123456),
            Some("minecraft:bowtie"),
        ),
        ("color reset", None, Some("minecraft:bowtie")),
        ("style reset", None, None),
        ("style set minecraft:default", None, None),
    ] {
        fixture.alice.take_packets();
        fixture.bob.take_packets();
        dispatcher
            .execute_input(
                &format!("waypoint modify {} {suffix}", mob.get_entity().entity_uuid),
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        let icon = living.waypoint_icon.snapshot();
        assert_eq!(icon.color, color);
        assert_eq!(icon.style.as_deref(), style);
        let mut saved = pumpkin_nbt::compound::NbtCompound::new();
        living.write_living_nbt(&mut saved);
        if matches!(suffix, "style reset" | "style set minecraft:default") {
            assert!(saved.get_compound("locator_bar_icon").is_none());
        }
        living.waypoint_icon.mutate(|icon| {
            *icon = crate::entity::living::waypoint_icon::LocatorBarIcon::default();
        });
        living.read_living_nbt_non_mut(&saved);
        let restored = living.waypoint_icon.snapshot();
        assert_eq!(restored.color, color);
        assert_eq!(restored.style.as_deref(), style);
        let a = waypoint_packets(&mut fixture.alice);
        let b = waypoint_packets(&mut fixture.bob);
        assert!(a.is_empty());
        assert!(b.is_empty());
    }
    // Neither a viewer leaving range nor a disconnected viewer can keep a frozen marker.
    fixture
        .alice
        .player
        .get_entity()
        .set_pos(Vector3::new(1000.0, 64.0, 0.0));
    fixture
        .world
        .players
        .store(Arc::new(vec![fixture.alice.player.clone()]));
    dispatcher
        .execute_input(
            &format!(
                "waypoint modify {} color hex 123456",
                mob.get_entity().entity_uuid
            ),
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    assert!(waypoint_packets(&mut fixture.alice).is_empty());
    assert!(waypoint_packets(&mut fixture.bob).is_empty());
    Ok(())
}

#[tokio::test]
async fn review3_waypoint_color_refreshes_connected_sender_with_untrack_then_track() -> TestResult {
    let mut fixture = Fixture::new()?;
    let mob = waypoint_in_range(&fixture);
    let source = fixture.sources()[0].clone();
    let dispatcher = fixture.server.command_dispatcher.load();
    fixture.alice.take_packets();
    fixture.bob.take_packets();
    let cases: [(&str, &[u8]); 4] = [
        (
            "color hex 123456",
            b"\x11minecraft:default\x01\x12\x34\x56\x01\x01\x40\x02",
        ),
        (
            "style set minecraft:bowtie",
            b"\x10minecraft:bowtie\x01\x12\x34\x56\x01\x01\x40\x02",
        ),
        ("color reset", b"\x10minecraft:bowtie\x00\x01\x01\x40\x02"),
        ("style reset", b"\x11minecraft:default\x00\x01\x01\x40\x02"),
    ];
    for (suffix, expected) in cases {
        dispatcher
            .execute_input(
                &format!("waypoint modify {} {suffix}", mob.get_entity().entity_uuid),
                &source,
            )
            .map_err(|error| format!("{error:?}"))?;
        let packets = waypoint_packets(&mut fixture.alice);
        assert_eq!(packets.len(), 2, "{suffix}");
        assert_waypoint_packet(
            &packets[0],
            1,
            mob.get_entity().entity_uuid,
            b"\x11minecraft:default\x00\x00",
        );
        assert_waypoint_packet(&packets[1], 0, mob.get_entity().entity_uuid, expected);
        assert!(waypoint_packets(&mut fixture.bob).is_empty());
    }
    // Entity.distanceTo rounds this just-inside position to the excluded range boundary.
    fixture
        .alice
        .player
        .get_entity()
        .set_pos(Vector3::new(100.999_999, 64.0, 2.0));
    dispatcher
        .execute_input(
            &format!("waypoint modify {} color red", mob.get_entity().entity_uuid),
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    let packets = waypoint_packets(&mut fixture.alice);
    assert_eq!(packets.len(), 1);
    assert_waypoint_packet(
        &packets[0],
        1,
        mob.get_entity().entity_uuid,
        b"\x11minecraft:default\x00\x00",
    );
    fixture
        .world
        .players
        .store(Arc::new(vec![fixture.bob.player.clone()]));
    dispatcher
        .execute_input(
            &format!(
                "waypoint modify {} color blue",
                mob.get_entity().entity_uuid
            ),
            &source,
        )
        .map_err(|error| format!("{error:?}"))?;
    assert!(waypoint_packets(&mut fixture.alice).is_empty());
    assert!(waypoint_packets(&mut fixture.bob).is_empty());
    Ok(())
}

#[tokio::test]
async fn signed_and_unsigned_msg_route_to_executing_player_and_honor_silence() -> TestResult {
    let mut fixture = Fixture::new()?;
    let mut recipient = TestPlayer::new(&fixture.world);
    recipient
        .player
        .get_entity()
        .add_scoreboard_tag("command_recipient");
    fixture.world.players.store(Arc::new(vec![
        fixture.alice.player.clone(),
        fixture.bob.player.clone(),
        recipient.player.clone(),
    ]));
    let source = fixture.sources()[0].clone();
    let dispatcher = fixture.server.command_dispatcher.load();
    for signed in [false, true] {
        for silent in [false, true] {
            let mut source = source.clone();
            source.silent = silent;
            if signed {
                let mut message = crate::net::chat::PlayerChatMessage::unsigned(
                    fixture.alice.player.get_entity().entity_uuid,
                    "hello".into(),
                );
                message.signature = Some(vec![5; 256].into_boxed_slice());
                source.signing_context = Arc::new(std::collections::HashMap::from([(
                    "message".into(),
                    message,
                )]));
            }
            fixture.alice.take_packets();
            fixture.bob.take_packets();
            recipient.take_packets();
            dispatcher.execute_input("execute as @a[tag=command_bob,limit=1] run msg @a[tag=command_recipient] hello", &source)
                .map_err(|error| format!("{error:?}"))?;
            let chat = if signed {
                pumpkin_data::packet::clientbound::play::PLAYER_CHAT.0
            } else {
                pumpkin_data::packet::clientbound::play::DISGUISED_CHAT.0
            };
            let alice = fixture.alice.take_packets();
            let bob = fixture.bob.take_packets();
            let incoming = recipient.take_packets();
            assert!(!alice.iter().any(|p| packet_id(p) == chat));
            assert_eq!(
                bob.iter().filter(|p| packet_id(p) == chat).count(),
                usize::from(!silent)
            );
            assert_eq!(incoming.iter().filter(|p| packet_id(p) == chat).count(), 1);
            if signed {
                let packet = incoming.iter().find(|p| packet_id(p) == chat).unwrap();
                let mut data = packet.as_ref();
                pumpkin_protocol::codec::var_int::VarInt::decode(&mut data).unwrap();
                pumpkin_protocol::codec::var_int::VarInt::decode(&mut data).unwrap();
                assert_eq!(
                    &data[..16],
                    fixture.alice.player.get_entity().entity_uuid.as_bytes()
                );
                assert!(data.windows(256).any(|w| w == [5; 256]));
            }
        }
    }
    Ok(())
}
