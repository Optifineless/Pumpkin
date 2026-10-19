use super::{lifecycle::LogBuffer, *};
use crate::{
    data::VanillaData,
    net::{ClientPlatform, GameProfile, PlayerConfig, java::combat_test_support::captured_client},
};
use arc_swap::ArcSwap;
use pumpkin_config::{AdvancedConfiguration, BasicConfiguration, TelemetryConfig};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{math::vector2::Vector2, version::JavaMinecraftVersion};
use pumpkin_world::world_info::{
    LevelData, WorldInfoWriter, anvil::AnvilLevelInfo, data_files::write_saved_data,
};
use std::{io::Cursor, process::Command};

pub(super) fn isolated_child(name: &str) -> bool {
    const CHILD: &str = "PUMPKIN_SCOREBOARD_SAVED_COMPONENT_CHILD";
    if std::env::var(CHILD).is_ok_and(|value| value == name) {
        return true;
    }
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([name, "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Saved-component regression child failed:\n{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    false
}

pub(super) async fn startup(path: &Path) -> (Arc<Server>, LogBuffer) {
    AnvilLevelInfo
        .write_world_info(&LevelData::default(pumpkin_util::world_seed::Seed(0)), path)
        .unwrap();
    let logs = LogBuffer(Arc::default());
    let captured = logs.clone();
    // Server::new loads saved data in spawn_blocking. A child-local global
    // subscriber also observes those workers without changing other tests' logs.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(move || captured.clone())
            .finish(),
    )
    .unwrap();
    let basic = BasicConfiguration {
        default_level_name: path.to_string_lossy().into_owned(),
        allow_nether: false,
        allow_end: false,
        allow_chat_reports: false,
        ..BasicConfiguration::default()
    };
    let mut advanced = AdvancedConfiguration::default();
    advanced.networking.bedrock.enabled = false;
    advanced.networking.java.online_mode = false;
    let data = VanillaData::load();
    let server = tokio::time::timeout(
        Duration::from_secs(60),
        Server::new(
            basic,
            advanced,
            TelemetryConfig::default(),
            data,
            Vec::new(),
        ),
    )
    .await
    .expect("Production server startup timed out")
    .unwrap();
    (server, logs)
}

pub(super) async fn joining_packets(server: &Arc<Server>) -> Vec<bytes::Bytes> {
    let profile = GameProfile {
        id: uuid::Uuid::new_v4(),
        name: "saved-component".into(),
        properties: ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    };
    let (client, mut receive) = captured_client(profile.clone());
    // This is the ReadyToPlay Java join path after authentication/configuration,
    // using the real packet serializer and the existing captured transport.
    let (joined, mut packets) = lifecycle::collect(
        server.add_player(
            Arc::new(ClientPlatform::Java(client)),
            profile,
            Some(PlayerConfig::default()),
        ),
        &mut receive,
    )
    .await;
    let (player, world) = joined.unwrap();
    player.client.java().unwrap().set_player(player.clone());
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let ((), spawn_packets) = lifecycle::collect(
        world.spawn_java_player(&server.basic_config, &player, server),
        &mut receive,
    )
    .await;
    packets.extend(spawn_packets);
    for id in [
        pumpkin_data::packet::clientbound::play::LOGIN.0,
        pumpkin_data::packet::clientbound::play::SET_DEFAULT_SPAWN_POSITION.0,
    ] {
        assert_eq!(
            packet_count(&packets, id),
            1,
            "Java join did not finish through the scoreboard synchronization point"
        );
    }
    assert_eq!(
        packet_count(
            &packets,
            pumpkin_data::packet::clientbound::play::DISCONNECT.0
        ),
        0,
        "Saved scoreboard prevented the Java client from joining"
    );
    lifecycle::collect(
        crate::net::java::connection_owner::finish_disconnect(&player, server),
        &mut receive,
    )
    .await;
    packets
}

pub(super) fn write_fixture(path: &Path, root: NbtCompound) -> Vec<u8> {
    let destination = path.join("data/minecraft/scoreboard.dat");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    write_saved_data(&destination, root).unwrap();
    let bytes = fs::read(destination).unwrap();
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "Fixture must be real gzip NBT");
    bytes
}

fn minimal_root(objective: NbtCompound, team: Option<NbtCompound>) -> NbtCompound {
    let mut data = NbtCompound::new();
    data.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    if let Some(team) = team {
        data.put_list("Teams", vec![NbtTag::Compound(team)]);
    }
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", 5023);
    root.put_compound("data", data);
    root
}

async fn rejects_malformed(root: NbtCompound) {
    let directory = tempfile::tempdir().unwrap();
    let original = write_fixture(directory.path(), root);
    let (server, logs) = startup(directory.path()).await;
    let empty = {
        let scoreboard = server.scoreboard.lock().unwrap();
        scoreboard.get_objectives().is_empty()
            && scoreboard.get_teams().is_empty()
            && scoreboard.get_scores().is_empty()
    };
    let packets = joining_packets(&server).await;
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add fresh dummy");
    let saved = tokio::time::timeout(Duration::from_secs(30), server.save_all())
        .await
        .expect("Real save after scoreboard mutation timed out");
    let preserved_after_save =
        fs::read(directory.path().join("data/minecraft/scoreboard.dat")).unwrap() == original;
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("ERROR")
            && log.contains("Failed to load scoreboard.dat")
            && log.contains("scoreboard saves disabled")
            && saved.is_err()
            && preserved_after_save
            && fs::read(directory.path().join("data/minecraft/scoreboard.dat")).unwrap()
                == original,
        "Malformed saved component must log an error and preserve original bytes after a real mutation/save and shutdown; save={saved:?}, preserved_after_save={preserved_after_save}, log={log}"
    );
    assert!(
        empty,
        "Failed scoreboard load retained partially loaded state"
    );
    for id in [
        pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
        pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0,
        pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0,
    ] {
        assert_eq!(
            packet_count(&packets, id),
            0,
            "Malformed scoreboard reached the joining Java client in packet {id}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_saved_component_scalar_objective_is_quarantined() {
    if !isolated_child("scoreboard_regression_saved_component_scalar_objective_is_quarantined") {
        return;
    }
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "bad".into());
    objective.put_int("DisplayName", 42);
    rejects_malformed(minimal_root(objective, None)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_saved_component_scalar_team_prefix_is_quarantined() {
    if !isolated_child("scoreboard_regression_saved_component_scalar_team_prefix_is_quarantined") {
        return;
    }
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "good".into());
    objective.put_string("DisplayName", "Good".into());
    let mut team = NbtCompound::new();
    team.put_string("Name", "bad_team".into());
    team.put_int("MemberNamePrefix", 42);
    rejects_malformed(minimal_root(objective, Some(team))).await;
}

pub(super) fn objective_components(packets: &[bytes::Bytes]) -> Vec<(String, NbtTag)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut input = Cursor::new(packet.as_ref());
            if input.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0
            {
                return None;
            }
            let name = input.get_str().unwrap().into_string();
            assert_eq!(
                input.get_u8().unwrap(),
                0,
                "Join must create objectives once"
            );
            Some((
                name,
                input
                    .get_nbt_with_version(&JavaMinecraftVersion::V_26_3)
                    .unwrap()
                    .unwrap(),
            ))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_saved_component_opaque_nbt_joins_and_round_trips() {
    if !isolated_child("scoreboard_regression_saved_component_opaque_nbt_joins_and_round_trips") {
        return;
    }
    // 26.3 NbtContents.MAP_CODEC: direct path and block source, with optional
    // separator/interpret; the style codec ignores unknown style fields.
    let mut opaque = NbtCompound::new();
    opaque.put_string("nbt", "CustomName".into());
    opaque.put_string("block", "0 64 0".into());
    opaque.put_bool("interpret", false);
    opaque.put_string("separator", "|".into());
    let mut future_style = NbtCompound::new();
    future_style.put_string("effect", "outline".into());
    opaque.put_compound("future_style", future_style);
    let opaque = NbtTag::Compound(opaque);
    let mut input = vanilla::fixture();
    let NbtTag::Compound(data) = input.child_tags.get_mut("data").unwrap() else {
        panic!("Vanilla fixture must contain data");
    };
    let objectives = data.child_tags.get_mut("Objectives").unwrap();
    let NbtTag::List(objectives) = objectives else {
        panic!("Vanilla fixture must contain objectives");
    };
    let NbtTag::Compound(objective) = &mut objectives[0] else {
        panic!("Vanilla fixture must contain objective compounds");
    };
    objective.put("DisplayName", opaque.clone());
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), input.clone());
    let (server, logs) = startup(directory.path()).await;
    let packets = joining_packets(&server).await;
    let objectives = objective_components(&packets);
    assert_eq!(objectives.len(), 2, "Join must send both saved objectives");
    assert_eq!(
        objectives
            .iter()
            .filter(|(name, _)| name == "blank")
            .count(),
        1
    );
    assert_eq!(
        objectives
            .iter()
            .filter(|(name, _)| name == "imported")
            .count(),
        1
    );
    assert_eq!(
        objectives
            .iter()
            .find(|(name, _)| name == "imported")
            .unwrap()
            .1,
        NbtTag::String("imported".into()),
        "The actual Java objective packet must project unsupported content to its objective name"
    );
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add temporary dummy");
    save(&server).await;
    command(&server, &world, "scoreboard objectives remove temporary");
    save(&server).await;
    vanilla::assert_semantic_payload_eq(
        payload(&input),
        payload(&read_scoreboard(directory.path())),
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(!log.contains("Failed to load scoreboard.dat"), "{log}");
}
