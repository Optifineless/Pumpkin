use super::{saved_components::*, *};
use crate::world::scoreboard::save_data_component_tests::examples;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::{text::component_codec, version::JavaMinecraftVersion};
use std::io::Cursor;

const VERSION: JavaMinecraftVersion = JavaMinecraftVersion::V_26_3;

fn format(value: NbtTag) -> NbtCompound {
    let mut format = NbtCompound::new();
    format.put_string("type", "minecraft:fixed".into());
    format.put("value", value);
    format
}

fn review_root() -> NbtCompound {
    let mut objectives = Vec::new();
    let mut scores = Vec::new();
    let mut teams = Vec::new();
    for (index, value) in examples().into_iter().enumerate() {
        let name = format!("review_{index}");
        let mut objective = NbtCompound::new();
        objective.put_string("Name", name.clone());
        objective.put("DisplayName", value.clone());
        objective.put_compound("format", format(value.clone()));
        objectives.push(NbtTag::Compound(objective));
        let mut score = NbtCompound::new();
        score.put_string("Name", "Holder".into());
        score.put_string("Objective", name.clone());
        score.put("display", value.clone());
        score.put_compound("format", format(value.clone()));
        scores.push(NbtTag::Compound(score));
        let mut team = NbtCompound::new();
        team.put_string("Name", name.clone());
        for field in ["DisplayName", "MemberNamePrefix", "MemberNameSuffix"] {
            team.put(field, value.clone());
        }
        teams.push(NbtTag::Compound(team));
        if let NbtTag::Compound(mut style) = value {
            style.put_string("type", "minecraft:styled".into());
            let mut objective = NbtCompound::new();
            objective.put_string("Name", format!("style_{index}"));
            objective.put_string("DisplayName", "Styled".into());
            objective.put_compound("format", style.clone());
            objectives.push(NbtTag::Compound(objective));
            let mut score = NbtCompound::new();
            score.put_string("Name", "StyledHolder".into());
            score.put_string("Objective", name);
            score.put_compound("format", style);
            scores.push(NbtTag::Compound(score));
        }
    }
    let mut data = NbtCompound::new();
    data.put_list("Objectives", objectives);
    data.put_list("PlayerScores", scores);
    data.put_list("Teams", teams);
    data.put_compound("DisplaySlots", NbtCompound::new());
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", 5023);
    root.put_compound("data", data);
    root
}

fn assert_subset(tag: &NbtTag, style_only: bool) {
    let NbtTag::Compound(data) = tag else {
        assert!(
            matches!(tag, NbtTag::String(_)) && !style_only,
            "Unsafe component: {tag:?}"
        );
        return;
    };
    if !style_only {
        assert!(data.get_string("text").is_some() || data.get_string("translate").is_some());
    }
    for (name, value) in &data.child_tags {
        match name.as_ref() {
            "text" | "translate" | "fallback" if !style_only => {
                assert!(value.extract_string().is_some());
            }
            "with" | "extra" if !style_only => {
                let children = value.extract_list().unwrap();
                assert!(!children.is_empty());
                for child in children {
                    assert_subset(child, false);
                }
            }
            "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" => {
                assert!(matches!(value, NbtTag::Byte(0 | 1)));
            }
            "color" => {
                let color = value.extract_string().unwrap();
                assert!(
                    pumpkin_util::text::color::NamedColor::try_from(color).is_ok()
                        || color.strip_prefix('#').is_some_and(
                            |hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                        ),
                    "Unsafe color in packet: {tag:?}"
                );
            }
            "font" => {
                let font = value.extract_string().unwrap();
                assert!(!font.starts_with("..:"));
                assert!(pumpkin_util::identifier::Identifier::parse(font).is_ok());
            }
            _ => panic!("Raw saved key {name:?} escaped to a client: {tag:?}"),
        }
    }
}

fn read_component(input: &mut Cursor<&[u8]>, style: bool) {
    assert_subset(
        &input.get_nbt_with_version(&VERSION).unwrap().unwrap(),
        style,
    );
}

fn read_format(input: &mut Cursor<&[u8]>) {
    if input.get_bool().unwrap() {
        match input.get_var_int().unwrap().0 {
            0 => {}
            1 => read_component(input, true),
            2 => read_component(input, false),
            value => panic!("Unexpected number-format ID {value}"),
        }
    }
}

fn assert_packet_subset(packets: &[bytes::Bytes], root: &NbtCompound) {
    for (id, list) in [
        (
            pumpkin_data::packet::clientbound::play::SCOREBOARD_OBJECTIVE.0,
            "Objectives",
        ),
        (
            pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0,
            "Teams",
        ),
        (
            pumpkin_data::packet::clientbound::play::UPDATE_SCORE.0,
            "PlayerScores",
        ),
    ] {
        assert_eq!(
            packet_count(packets, id),
            payload(root).get_list(list).unwrap().len()
        );
        for packet in packets {
            let mut input = Cursor::new(packet.as_ref());
            if input.get_var_int().unwrap().0 != id {
                continue;
            }
            input.get_str().unwrap();
            match list {
                "Objectives" => {
                    assert_eq!(input.get_u8().unwrap(), 0);
                    read_component(&mut input, false);
                    input.get_var_int().unwrap();
                    read_format(&mut input);
                }
                "Teams" => {
                    assert_eq!(input.get_u8().unwrap(), 0);
                    for _ in 0..3 {
                        read_component(&mut input, false);
                    }
                }
                _ => {
                    input.get_str().unwrap();
                    input.get_var_int().unwrap();
                    if input.get_bool().unwrap() {
                        read_component(&mut input, false);
                    }
                    read_format(&mut input);
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_review_components_start_join_project_and_save_losslessly() {
    if !isolated_child(
        "scoreboard_regression_review_components_start_join_project_and_save_losslessly",
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = review_root();
    write_fixture(directory.path(), root.clone());
    let (server, logs) = startup(directory.path()).await;
    let packets = joining_packets(&server).await;
    assert_packet_subset(&packets, &root);
    // This covers command feedback and Bedrock's plain-text title source as well.
    let board = server.scoreboard.lock().unwrap();
    for objective in board.get_objectives().values() {
        assert_subset(
            &component_codec::write_component(&objective.display_name),
            false,
        );
    }
    assert_eq!(
        board.get_objectives()["review_3"]
            .display_name
            .clone()
            .get_text(),
        "review_3"
    );
    drop(board);
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add temporary dummy");
    save(&server).await;
    command(&server, &world, "scoreboard objectives remove temporary");
    save(&server).await;
    vanilla::assert_semantic_payload_eq(
        payload(&root),
        payload(&read_scoreboard(directory.path())),
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    let log = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(!log.contains("Failed to load scoreboard.dat"), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_command_subset_projects_unchanged() {
    if !isolated_child("scoreboard_regression_command_subset_projects_unchanged") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let (server, _) = startup(directory.path()).await;
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "scoreboard objectives add safe dummy");
    command(
        &server,
        &world,
        r#"scoreboard objectives modify safe displayname {"text":"Safe","bold":true,"color":"red"}"#,
    );
    command(
        &server,
        &world,
        "scoreboard objectives add empty_session dummy",
    );
    command(
        &server,
        &world,
        r#"scoreboard objectives modify empty_session displayname {"text":""}"#,
    );
    command(&server, &world, "team add safe_team");
    // Team commands currently take literal strings. Exercise a command-created team
    // with a session-built translated prefix without changing that existing syntax.
    let mut prefix = NbtCompound::new();
    prefix.put_string("translate", "custom.prefix".into());
    prefix.put_string("fallback", "%s: %s".into());
    prefix.put_list(
        "with",
        vec![NbtTag::String("A".into()), NbtTag::String("B".into())],
    );
    let prefix = component_codec::read_component(&NbtTag::Compound(prefix)).unwrap();
    let effects = crate::world::guard_effects::GuardEffects::new();
    let mut board = server.scoreboard.lock().unwrap();
    let mut team = board.get_team("safe_team").unwrap().clone();
    team.player_prefix = prefix;
    board.update_team(&world, team);
    drop(board);
    drop(effects);
    let (expected_objective, expected_prefix) = {
        let board = server.scoreboard.lock().unwrap();
        (
            component_codec::write_component(&board.get_objectives()["safe"].display_name),
            component_codec::write_component(&board.get_team("safe_team").unwrap().player_prefix),
        )
    };
    let expected_style = expected_objective.extract_compound().unwrap();
    assert_eq!(expected_style.get_string("text"), Some("Safe"));
    assert_eq!(expected_style.get_string("color"), Some("red"));
    assert_eq!(expected_style.get_bool("bold"), Some(true));
    let translated = expected_prefix.extract_compound().unwrap();
    assert_eq!(translated.get_string("translate"), Some("custom.prefix"));
    assert_eq!(translated.get_string("fallback"), Some("%s: %s"));
    assert_eq!(
        translated.get_list("with").unwrap(),
        &[NbtTag::String("A".into()), NbtTag::String("B".into())]
    );
    let packets = joining_packets(&server).await;
    let objectives: std::collections::BTreeMap<_, _> =
        objective_components(&packets).into_iter().collect();
    assert_eq!(objectives.len(), 2);
    assert_eq!(objectives["safe"], expected_objective);
    assert_eq!(objectives["empty_session"], NbtTag::String("".into()));
    let team = packets
        .iter()
        .find_map(|packet| {
            let mut input = Cursor::new(packet.as_ref());
            (input.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SET_PLAYER_TEAM.0)
                .then_some(input)
        })
        .unwrap();
    let mut input = team;
    assert_eq!(&*input.get_str().unwrap(), "safe_team");
    assert_eq!(input.get_u8().unwrap(), 0);
    read_component(&mut input, false);
    assert_eq!(
        input.get_nbt_with_version(&VERSION).unwrap().unwrap(),
        expected_prefix
    );
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}
