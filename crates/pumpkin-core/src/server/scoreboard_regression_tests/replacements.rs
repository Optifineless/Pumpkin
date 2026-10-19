use super::{saved_components::*, *};
use pumpkin_nbt::tag::NbtTag;

fn imported_text() -> NbtTag {
    let mut event = NbtCompound::new();
    event.put_string("action", "run_command".into());
    event.put_string("command", "/old".into());
    let mut text = NbtCompound::new();
    text.put_string("text", "A".into());
    text.put_compound("click_event", event);
    NbtTag::Compound(text)
}

fn fixture() -> NbtCompound {
    let mut root = vanilla::fixture();
    let NbtTag::Compound(data) = root.child_tags.get_mut("data").unwrap() else {
        panic!("Missing fixture data")
    };
    let teams = ["saved", "unsaved", "untouched", "equal"]
        .into_iter()
        .map(|name| {
            let mut team = NbtCompound::new();
            team.put_string("Name", name.into());
            for field in ["DisplayName", "MemberNamePrefix", "MemberNameSuffix"] {
                team.put(field, imported_text());
            }
            NbtTag::Compound(team)
        })
        .collect();
    data.put_list("Teams", teams);
    root
}

fn assert_replacements(root: &NbtCompound) {
    for name in ["saved", "unsaved", "equal", "untouched"] {
        let team = payload(root)
            .get_list("Teams")
            .unwrap()
            .iter()
            .filter_map(NbtTag::extract_compound)
            .find(|team| team.get_string("Name") == Some(name))
            .unwrap();
        assert_eq!(
            team.get("MemberNamePrefix"),
            Some(&if name == "untouched" {
                imported_text()
            } else {
                NbtTag::String("A".into())
            }),
            "Explicit {name} prefix replacement resurrected raw fields"
        );
        for field in ["DisplayName", "MemberNameSuffix"] {
            assert_eq!(team.get(field), Some(&imported_text()));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_component_replacements_do_not_resurrect_raw_fields() {
    if !isolated_child("scoreboard_regression_component_replacements_do_not_resurrect_raw_fields") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path(), fixture());
    let (server, _) = startup(directory.path()).await;
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "team modify saved prefix B");
    save(&server).await;
    command(&server, &world, "team modify saved prefix A");
    command(&server, &world, "team modify unsaved prefix B");
    command(&server, &world, "team modify unsaved prefix A");
    command(&server, &world, "team modify equal prefix A");
    command(&server, &world, "team modify untouched color blue");
    command(
        &server,
        &world,
        "scoreboard players set ImportedHolder imported 43",
    );
    save(&server).await;
    assert_replacements(&read_scoreboard(directory.path()));
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
    let (board, _) = super::super::scoreboard_storage::load_scoreboard(directory.path());
    assert_replacements(&{
        let mut root = NbtCompound::new();
        root.put_compound("data", board.lock().unwrap().to_save_data());
        root
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_equal_display_name_commands_replace_imported_events() {
    if !isolated_child("scoreboard_regression_equal_display_name_commands_replace_imported_events")
    {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut root = fixture();
    let NbtTag::Compound(data) = root.child_tags.get_mut("data").unwrap() else {
        panic!("Missing fixture data")
    };
    let NbtTag::List(objectives) = data.child_tags.get_mut("Objectives").unwrap() else {
        panic!("Missing objectives")
    };
    let NbtTag::Compound(objective) = &mut objectives[0] else {
        panic!("Missing objective")
    };
    objective.put("DisplayName", imported_text());
    write_fixture(directory.path(), root);
    let (server, _) = startup(directory.path()).await;
    let world = server.worlds.load()[0].clone();
    command(&server, &world, "team modify equal displayName A");
    command(
        &server,
        &world,
        r#"scoreboard objectives modify imported displayname {"text":"A"}"#,
    );
    save(&server).await;
    let saved = read_scoreboard(directory.path());
    let data = payload(&saved);
    let objective = data
        .get_list("Objectives")
        .unwrap()
        .iter()
        .filter_map(NbtTag::extract_compound)
        .find(|record| record.get_string("Name") == Some("imported"))
        .unwrap();
    assert_eq!(
        objective.get("DisplayName"),
        Some(&NbtTag::String("A".into()))
    );
    let team = data
        .get_list("Teams")
        .unwrap()
        .iter()
        .filter_map(NbtTag::extract_compound)
        .find(|record| record.get_string("Name") == Some("equal"))
        .unwrap();
    assert_eq!(team.get("DisplayName"), Some(&NbtTag::String("A".into())));
    assert_eq!(team.get("MemberNamePrefix"), Some(&imported_text()));
    server.shutdown().await;
    world.level.world_portal.store(Arc::new(None));
}
