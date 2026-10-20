use super::*;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_world::world_info::data_files::write_saved_data;

fn text(value: &str) -> NbtTag {
    // ComponentSerialization.CODEC collapses an unstyled literal to a string.
    NbtTag::String(value.into())
}

fn format(kind: &str) -> NbtCompound {
    let mut format = NbtCompound::new();
    format.put_string("type", format!("minecraft:{kind}"));
    format
}

pub(super) fn fixture() -> NbtCompound {
    // 26.3 ScoreboardSaveData.Packed.CODEC delegates to Objective.Packed,
    // Scoreboard.PackedScore, PlayerTeam.Packed and DisplaySlot.CODEC.
    // Components are direct NBT values; NumberFormatTypes uses the `type` key.
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "imported".into());
    objective.put_string("CriteriaName", "minecraft.custom:minecraft.jump".into());
    objective.put("DisplayName", text("Vanilla total"));
    objective.put_string("RenderType", "hearts".into());
    objective.put_bool("display_auto_update", true);
    let mut fixed = format("fixed");
    fixed.put("value", text("many"));
    objective.put_compound("format", fixed);

    let mut blank = NbtCompound::new();
    blank.put_string("Name", "blank".into());
    blank.put("DisplayName", text("Blank"));
    blank.put_compound("format", format("blank"));

    let mut score = NbtCompound::new();
    score.put_string("Name", "ImportedHolder".into());
    score.put_string("Objective", "imported".into());
    score.put_int("Score", 42);
    score.put_bool("Locked", true);
    score.put("display", text("Imported alias"));
    let mut styled = format("styled");
    styled.put_string("color", "gold".into());
    styled.put_bool("bold", true);
    styled.put_bool("italic", false);
    score.put_compound("format", styled);

    let mut team = NbtCompound::new();
    team.put_string("Name", "imported_red".into());
    team.put("DisplayName", text("Imported Red"));
    team.put_string("TeamColor", "red".into());
    team.put_bool("AllowFriendlyFire", false);
    team.put_bool("SeeFriendlyInvisibles", false);
    team.put("MemberNamePrefix", text("Prefix"));
    team.put("MemberNameSuffix", text("Suffix"));
    team.put_string("NameTagVisibility", "hideForOtherTeams".into());
    team.put_string("DeathMessageVisibility", "hideForOwnTeam".into());
    team.put_string("CollisionRule", "pushOtherTeams".into());
    team.put_list(
        "Players",
        vec![
            NbtTag::String("ImportedHolder".into()),
            NbtTag::String("OfflineMember".into()),
        ],
    );

    let mut slots = NbtCompound::new();
    slots.put_string("list", "imported".into());
    slots.put_string("sidebar", "imported".into());
    slots.put_string("below_name", "imported".into());
    for color in [
        "black",
        "dark_blue",
        "dark_green",
        "dark_aqua",
        "dark_red",
        "dark_purple",
        "gold",
        "gray",
        "dark_gray",
        "blue",
        "green",
        "aqua",
        "red",
        "light_purple",
        "yellow",
        "white",
    ] {
        slots.put_string(&format!("sidebar.team.{color}"), "imported".into());
    }
    let mut data = NbtCompound::new();
    data.put_list(
        "Objectives",
        vec![NbtTag::Compound(objective), NbtTag::Compound(blank)],
    );
    data.put_list("PlayerScores", vec![NbtTag::Compound(score)]);
    data.put_compound("DisplaySlots", slots);
    data.put_list("Teams", vec![NbtTag::Compound(team)]);
    let mut root = NbtCompound::new();
    root.put_int("DataVersion", 5023);
    root.put_compound("data", data);
    root
}

fn canonical(mut data: NbtCompound) -> NbtCompound {
    for key in ["Objectives", "PlayerScores", "Teams"] {
        if let Some(NbtTag::List(list)) = data.child_tags.get_mut(key) {
            list.sort_by_key(|entry| {
                let entry = entry.extract_compound().unwrap();
                (
                    entry.get_string("Name").map(str::to_owned),
                    entry.get_string("Objective").map(str::to_owned),
                )
            });
        }
    }
    data
}

pub(super) fn assert_semantic_payload_eq(expected: &NbtCompound, actual: &NbtCompound) {
    assert_eq!(canonical(expected.clone()), canonical(actual.clone()));
}

fn assert_imported_state(world: &World) {
    let scoreboard = world.scoreboard.lock().unwrap();
    assert!(
        scoreboard.get_objective("imported").is_some(),
        "Startup ignored vanilla scoreboard.dat"
    );
    assert_eq!(
        scoreboard.get_objective("imported").unwrap().criterion,
        "minecraft.custom:minecraft.jump"
    );
    assert_eq!(
        scoreboard.get_score_value("ImportedHolder", "imported"),
        Some(42)
    );
    assert_eq!(
        scoreboard.get_team("imported_red").unwrap().players.len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoreboard_regression_vanilla_26_3_file_load_save_preserves_every_codec_field() {
    let directory = tempfile::tempdir().unwrap();
    let input = fixture();
    fs::create_dir_all(directory.path().join("data/minecraft")).unwrap();
    write_saved_data(
        &directory.path().join("data/minecraft/scoreboard.dat"),
        input.clone(),
    )
    .unwrap();
    with_worlds(directory.path(), None, |server, worlds| async move {
        assert_imported_state(&worlds[2]);
        // Ensure this is a real re-encode, rather than a clean-file no-op.
        command(
            &server,
            &worlds[1],
            "scoreboard objectives add temporary dummy",
        );
        save(&server).await;
        command(
            &server,
            &worlds[1],
            "scoreboard objectives remove temporary",
        );
        save(&server).await;
        let saved = read_scoreboard(&server.basic_config.get_world_path());
        assert_eq!(saved.get_int("DataVersion"), Some(5023));
        assert_semantic_payload_eq(payload(&input), payload(&saved));
    })
    .await;
}
