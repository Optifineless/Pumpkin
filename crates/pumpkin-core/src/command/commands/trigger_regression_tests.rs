use super::super::CommandSender;
use crate::{
    net::{GameProfile, java::combat_test_support::TestPlayer},
    server::{Server, combat_test_support},
    world::{World, scoreboard::ScoreboardScore},
};
use arc_swap::ArcSwap;
use pumpkin_nbt::{compound::NbtCompound, nbt_compress::read_gzip_compound_tag, tag::NbtTag};
use pumpkin_protocol::{
    ClientPacket, NumberFormat, codec::var_int::VarInt, java::client::play::CUpdateScore,
    ser::NetworkReadExt,
};
use pumpkin_util::{PermissionLvl, math::vector2::Vector2, version::JavaMinecraftVersion};
use pumpkin_world::world_info::data_files::write_saved_data;
use std::{fs, path::Path, sync::Arc, time::Duration};

const PLAYER_NAME: &str = "TriggerOwner";
const OBSERVER_NAME: &str = "JavaObserver";

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    observer: TestPlayer,
}

impl Fixture {
    fn new(path: &Path) -> Self {
        let server = combat_test_support::server(path);
        let world = combat_test_support::world(&server, path);
        let mut player = TestPlayer::with_profile(&world, profile(PLAYER_NAME));
        let mut observer = TestPlayer::with_profile(&world, profile(OBSERVER_NAME));
        player.player.permission_lvl.store(PermissionLvl::Four);
        world.players.store(Arc::new(vec![
            player.player.clone(),
            observer.player.clone(),
        ]));
        server.worlds.store(Arc::new(vec![world.clone()]));
        player.player.send_scoreboard();
        observer.player.send_scoreboard();
        player.take_packets();
        observer.take_packets();
        Self {
            server,
            world,
            player,
            observer,
        }
    }

    fn execute_player(&self, input: &str) -> Result<i32, String> {
        self.server
            .command_dispatcher
            .load()
            .execute_input(input, &self.player.player.get_command_source(&self.server))
            .map_err(|error| format!("{error:?}"))
    }

    fn run_player(&self, input: &str) -> i32 {
        self.execute_player(input)
            .unwrap_or_else(|error| panic!("Command {input:?} failed: {error}"))
    }

    fn execute_console(&self, input: &str) -> Result<i32, String> {
        let source = CommandSender::Console
            .into_source(&self.server)
            .with_world(self.world.clone());
        self.server
            .command_dispatcher
            .load()
            .execute_input(input, &source)
            .map_err(|error| format!("{error:?}"))
    }

    fn create_trigger(&self, objective: &str, value: i32, display: &str, number_format: &str) {
        self.run_player(&format!("scoreboard objectives add {objective} trigger"));
        self.run_player(&format!(
            "scoreboard players set {PLAYER_NAME} {objective} {value}"
        ));
        self.run_player(&format!(
            "scoreboard players display name {PLAYER_NAME} {objective} {{\"text\":\"{display}\",\"color\":\"gold\",\"italic\":true}}"
        ));
        self.run_player(&format!(
            "scoreboard players display numberformat {PLAYER_NAME} {objective} {number_format}"
        ));
    }

    fn score(&self, objective: &str) -> ScoreboardScore {
        self.world
            .scoreboard
            .lock()
            .unwrap()
            .get_score(PLAYER_NAME, objective)
            .unwrap()
            .clone()
    }

    fn score_updates(&mut self) -> Vec<bytes::Bytes> {
        self.observer
            .take_packets()
            .into_iter()
            .filter(|packet| {
                packet_id(packet) == pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
            })
            .collect()
    }

    async fn shutdown(self) {
        for player in [&self.player, &self.observer] {
            player.client().close();
            self.world
                .entity_tracker
                .remove_entity(player.player.as_ref(), &self.world);
        }
        self.world.players.store(Arc::new(Vec::new()));
        tokio::time::timeout(Duration::from_secs(30), self.server.shutdown())
            .await
            .expect("Test server shutdown timed out");
        self.world.level.world_portal.store(Arc::new(None));
    }
}

fn profile(name: &str) -> GameProfile {
    GameProfile {
        id: uuid::Uuid::new_v4(),
        name: name.into(),
        properties: ArcSwap::from_pointee(Vec::new()),
        profile_actions: None,
    }
}

fn packet_id(mut packet: &[u8]) -> i32 {
    VarInt::decode(&mut packet).unwrap().0
}

fn expected_score_packet(score: &ScoreboardScore) -> bytes::Bytes {
    CUpdateScore::new(
        score.entity_name.clone(),
        score.objective_name.clone(),
        score.value,
        score.display_name.clone(),
        score.number_format.clone(),
    )
    .serialize_packet(&JavaMinecraftVersion::V_26_3)
    .unwrap()
}

#[derive(Debug, PartialEq)]
struct ParsedScorePacket {
    entity_name: String,
    objective_name: String,
    value: i32,
    display: Option<NbtTag>,
    number_format: Option<(i32, Option<NbtTag>)>,
}

fn parsed_score_packet(packet: &[u8]) -> ParsedScorePacket {
    let mut input = packet;
    assert_eq!(
        input.get_var_int().unwrap().0,
        pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0
    );
    let entity_name = input.get_str().unwrap().into_string();
    let objective_name = input.get_str().unwrap().into_string();
    let value = input.get_var_int().unwrap().0;
    let version = JavaMinecraftVersion::V_26_3;
    let display = input
        .get_option(|reader| reader.get_nbt_with_version(&version))
        .unwrap()
        .flatten();
    let number_format = input
        .get_option(|reader| {
            let kind = reader.get_var_int()?.0;
            let format = if kind == 0 {
                None
            } else {
                reader.get_nbt_with_version(&version)?
            };
            Ok((kind, format))
        })
        .unwrap();
    assert!(input.is_empty(), "score packet had trailing bytes");
    ParsedScorePacket {
        entity_name,
        objective_name,
        value,
        display,
        number_format,
    }
}

fn assert_score_packet_matches(packets: &[bytes::Bytes], expected: &ScoreboardScore) {
    assert_eq!(
        packets.len(),
        1,
        "successful trigger must publish one score update"
    );
    assert_eq!(
        parsed_score_packet(&packets[0]),
        parsed_score_packet(&expected_score_packet(expected))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g07_trigger_variants_preserve_display_and_number_format() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(directory.path());
    let setups = [
        (
            "simple_fmt",
            6,
            "Simple alias",
            "fixed {\"text\":\"simple ticks\"}",
            "trigger simple_fmt",
            7,
        ),
        (
            "add_fmt",
            8,
            "Add alias",
            "styled {color:'blue',bold:true}",
            "trigger add_fmt add 4",
            12,
        ),
        (
            "set_fmt",
            3,
            "Set alias",
            "fixed {\"text\":\"set points\",\"bold\":true}",
            "trigger set_fmt set -5",
            -5,
        ),
    ];
    for (objective, value, display, format, _, _) in setups {
        fixture.create_trigger(objective, value, display, format);
    }
    for (objective, _, _, _, _, _) in setups {
        fixture.run_player(&format!(
            "scoreboard players enable {PLAYER_NAME} {objective}"
        ));
    }
    fixture.run_player("scoreboard objectives add plain trigger");
    fixture.run_player(&format!("scoreboard players set {PLAYER_NAME} plain 9"));
    fixture.run_player(&format!("scoreboard players enable {PLAYER_NAME} plain"));

    let mut observations = Vec::new();
    for (objective, _, _, _, command, expected_value) in setups {
        fixture.run_player(&format!(
            "scoreboard objectives setdisplay sidebar {objective}"
        ));
        fixture.observer.take_packets();
        let before = fixture.score(objective);
        let result = fixture.run_player(command);
        let after = fixture.score(objective);
        let packets = fixture.score_updates();
        observations.push((before, after, result, expected_value, packets));
    }

    fixture.run_player("scoreboard objectives setdisplay sidebar plain");
    fixture.observer.take_packets();
    let plain_result = fixture.run_player("trigger plain");
    let plain_after = fixture.score("plain");
    let plain_packets = fixture.score_updates();

    for (before, after, result, expected_value, _) in &observations {
        assert_eq!(*result, *expected_value);
        assert_eq!(after.value.0, *expected_value);
        assert!(after.locked, "successful trigger must lock its score");
        assert!(!before.locked, "test setup must prime each score");
    }
    assert_eq!(plain_result, 10);
    assert_eq!(plain_after.value.0, 10);
    assert!(plain_after.locked);
    assert!(plain_after.display_name.is_none());
    assert!(plain_after.number_format.is_none());
    assert_score_packet_matches(&plain_packets, &plain_after);

    for (before, after, _, _, packets) in observations {
        assert!(before.display_name.is_some());
        assert!(before.number_format.is_some());
        assert_eq!(after.display_name, before.display_name);
        assert_eq!(after.number_format, before.number_format);
        assert_score_packet_matches(&packets, &after);
    }
    assert!(matches!(
        fixture.score("simple_fmt").number_format,
        Some(NumberFormat::Fixed(_))
    ));
    assert!(matches!(
        fixture.score("add_fmt").number_format,
        Some(NumberFormat::Styled(_))
    ));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g07_trigger_locked_and_invalid_controls_keep_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(directory.path());
    fixture.create_trigger("guarded", 14, "Guard alias", "fixed {\"text\":\"guarded\"}");
    fixture.run_player("scoreboard objectives add not_trigger dummy");
    fixture.run_player(&format!(
        "scoreboard players set {PLAYER_NAME} not_trigger 27"
    ));
    fixture.run_player(&format!(
        "scoreboard players display name {PLAYER_NAME} not_trigger {{\"text\":\"Dummy alias\"}}"
    ));
    fixture.run_player(&format!(
        "scoreboard players display numberformat {PLAYER_NAME} not_trigger styled {{color:'red'}}"
    ));
    fixture.run_player("scoreboard objectives setdisplay sidebar guarded");
    fixture.observer.take_packets();
    let before = fixture.score("guarded");
    let not_trigger_before = fixture.score("not_trigger");

    let locked = fixture
        .execute_player("trigger guarded")
        .expect_err("a locked trigger must fail");
    assert!(
        locked.contains("commands.trigger.failed.unprimed"),
        "{locked}"
    );
    let missing = fixture
        .execute_player("trigger missing")
        .expect_err("a missing objective must fail");
    assert!(
        missing.contains("commands.trigger.failed.invalid"),
        "{missing}"
    );
    let wrong_criterion = fixture
        .execute_player("trigger not_trigger")
        .expect_err("a non-trigger objective must fail");
    assert!(
        wrong_criterion.contains("commands.trigger.failed.invalid"),
        "{wrong_criterion}"
    );
    let console = fixture
        .execute_console("trigger guarded")
        .expect_err("console must not execute player-only trigger commands");
    assert!(console.contains("permissions.requires.player"), "{console}");
    assert!(fixture.score_updates().is_empty());
    assert_eq!(fixture.score("guarded"), before);
    assert_eq!(fixture.score("not_trigger"), not_trigger_before);

    fixture.run_player(&format!("scoreboard players enable {PLAYER_NAME} guarded"));
    fixture.observer.take_packets();
    assert_eq!(fixture.run_player("trigger guarded"), 15);
    let first_success = fixture.score("guarded");
    assert_eq!(first_success.value.0, 15);
    assert!(first_success.locked);
    assert_score_packet_matches(&fixture.score_updates(), &first_success);

    let locked_again = fixture
        .execute_player("trigger guarded")
        .expect_err("a successful trigger must lock the score");
    assert!(
        locked_again.contains("commands.trigger.failed.unprimed"),
        "{locked_again}"
    );
    assert!(fixture.score_updates().is_empty());
    assert_eq!(fixture.score("guarded"), first_success);

    fixture.run_player(&format!("scoreboard players enable {PLAYER_NAME} guarded"));
    fixture.observer.take_packets();
    assert_eq!(fixture.run_player("trigger guarded"), 16);
    let second_success = fixture.score("guarded");
    assert_eq!(second_success.value.0, 16);
    assert!(second_success.locked);
    assert_score_packet_matches(&fixture.score_updates(), &second_success);
    assert_eq!(second_success.display_name, before.display_name);
    assert_eq!(second_success.number_format, before.number_format);
    fixture.shutdown().await;
}

fn text_component(text: &str, color: &str) -> NbtTag {
    let mut component = NbtCompound::new();
    component.put_string("text", text.into());
    component.put_string("color", color.into());
    component.put_bool("italic", true);
    NbtTag::Compound(component)
}

fn fixed_format(text: &str) -> NbtCompound {
    let mut format = NbtCompound::new();
    format.put_string("type", "minecraft:fixed".into());
    format.put("value", text_component(text, "green"));
    format
}

fn styled_format(color: &str) -> NbtCompound {
    let mut format = NbtCompound::new();
    format.put_string("type", "minecraft:styled".into());
    format.put_string("color", color.into());
    format.put_bool("bold", true);
    format
}

fn saved_score(name: &str, value: i32, display: &str, format: NbtCompound) -> NbtCompound {
    let mut score = NbtCompound::new();
    score.put_string("Name", name.into());
    score.put_string("Objective", "persisted_trigger".into());
    score.put_int("Score", value);
    score.put_bool("Locked", false);
    score.put("display", text_component(display, "gold"));
    score.put_compound("format", format);
    let mut opaque = NbtCompound::new();
    opaque.put_string("future_marker", format!("keep-{name}"));
    opaque.put_long("future_number", 9_007_199_254_740_993);
    score.put_compound("future_component", opaque);
    score
}

fn saved_score_for<'a>(root: &'a NbtCompound, name: &str) -> &'a NbtCompound {
    root.get_compound("data")
        .unwrap()
        .get_list("PlayerScores")
        .unwrap()
        .iter()
        .filter_map(NbtTag::extract_compound)
        .find(|score| score.get_string("Name") == Some(name))
        .unwrap()
}

struct PersistedFixture {
    root: NbtCompound,
    target: NbtCompound,
    unchanged: NbtCompound,
}

fn persisted_fixture() -> PersistedFixture {
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "persisted_trigger".into());
    objective.put_string("CriteriaName", "trigger".into());
    objective.put("DisplayName", text_component("Saved trigger", "gold"));
    objective.put_string("RenderType", "integer".into());
    let mut unchanged = saved_score(
        "UnchangedHolder",
        48,
        "Control alias",
        styled_format("blue"),
    );
    unchanged.put_bool("Locked", true);
    let target = saved_score(
        PLAYER_NAME,
        31,
        "Saved alias",
        fixed_format("Saved fixed format"),
    );
    let mut data = NbtCompound::new();
    data.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    data.put_list(
        "PlayerScores",
        vec![
            NbtTag::Compound(target.clone()),
            NbtTag::Compound(unchanged.clone()),
        ],
    );
    let mut slots = NbtCompound::new();
    slots.put_string("sidebar", "persisted_trigger".into());
    data.put_compound("DisplaySlots", slots);
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", 5023);
    root.put_compound("data", data);

    PersistedFixture {
        root,
        target,
        unchanged,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g07_trigger_value_change_preserves_saved_components() {
    let directory = tempfile::tempdir().unwrap();
    let persisted = persisted_fixture();
    let target = persisted.target;
    let unchanged = persisted.unchanged;
    fs::create_dir_all(directory.path().join("data/minecraft")).unwrap();
    write_saved_data(
        &directory.path().join("data/minecraft/scoreboard.dat"),
        persisted.root,
    )
    .unwrap();

    let mut fixture = Fixture::new(directory.path());
    combat_test_support::publish_empty_chunk(&fixture.world, Vector2::new(0, 0));
    fixture.player.player.send_scoreboard();
    fixture.observer.player.send_scoreboard();
    fixture.observer.take_packets();
    let before = fixture.score("persisted_trigger");
    assert_eq!(before.value.0, 31);
    assert!(!before.locked);
    assert!(before.display_name.is_some());
    assert!(matches!(before.number_format, Some(NumberFormat::Fixed(_))));

    assert_eq!(fixture.run_player("trigger persisted_trigger"), 32);
    let after = fixture.score("persisted_trigger");
    assert_eq!(after.value.0, 32);
    assert!(after.locked);
    let packets = fixture.score_updates();

    tokio::time::timeout(Duration::from_secs(30), fixture.server.save_all())
        .await
        .expect("Real scoreboard save timed out")
        .unwrap();
    let save_path = directory.path().join("data/minecraft/scoreboard.dat");
    let saved = read_gzip_compound_tag(fs::File::open(&save_path).unwrap()).unwrap();
    let saved_control = saved_score_for(&saved, "UnchangedHolder");
    assert_eq!(saved_control, &unchanged);
    let saved_target = saved_score_for(&saved, PLAYER_NAME);
    assert_eq!(saved_target.get_int("Score"), Some(32));
    assert_eq!(saved_target.get_bool("Locked"), Some(true));
    assert_eq!(
        saved_target.get("display"),
        target.get("display"),
        "value-only trigger save must retain the original display component"
    );
    assert_eq!(
        saved_target.get("format"),
        target.get("format"),
        "value-only trigger save must retain the original number format"
    );
    assert_eq!(
        saved_target.get("future_component"),
        target.get("future_component"),
        "value-only trigger save must retain unknown score payload"
    );
    let mut expected_target = target;
    expected_target.put_int("Score", 32);
    expected_target.put_bool("Locked", true);
    assert_eq!(saved_target, &expected_target);
    assert_eq!(after.display_name, before.display_name);
    assert_eq!(after.number_format, before.number_format);
    let mut expected_packet_score = before.clone();
    expected_packet_score.value = VarInt(32);
    expected_packet_score.locked = true;
    assert_score_packet_matches(&packets, &expected_packet_score);

    fixture.shutdown().await;
    let reloaded = combat_test_support::server(directory.path());
    let reloaded_score = reloaded
        .scoreboard
        .lock()
        .unwrap()
        .get_score(PLAYER_NAME, "persisted_trigger")
        .unwrap()
        .clone();
    assert_eq!(reloaded_score.value.0, 32);
    assert!(reloaded_score.locked);
    assert_eq!(reloaded_score.display_name, before.display_name);
    assert_eq!(reloaded_score.number_format, before.number_format);
    let reloaded_control = reloaded
        .scoreboard
        .lock()
        .unwrap()
        .get_score("UnchangedHolder", "persisted_trigger")
        .unwrap()
        .clone();
    assert_eq!(reloaded_control.value.0, 48);
    assert!(reloaded_control.locked);
    tokio::time::timeout(Duration::from_secs(30), reloaded.shutdown())
        .await
        .expect("Reloaded test server shutdown timed out");
}
