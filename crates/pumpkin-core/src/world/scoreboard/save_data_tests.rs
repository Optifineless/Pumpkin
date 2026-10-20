use std::error::Error;

use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::client::play::RenderType;

use super::{CollisionRule, NameTagVisibility, NoTarget, Scoreboard};

// Objective.Packed, Score.Packed and PlayerTeam.Packed omit these default fields.
fn minimal_payload() -> NbtCompound {
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "points".into());
    objective.put_string("DisplayName", "Points".into());
    let mut score = NbtCompound::new();
    score.put_string("Name", "holder".into());
    score.put_string("Objective", "points".into());
    let mut team = NbtCompound::new();
    team.put_string("Name", "red".into());
    let mut data = NbtCompound::new();
    data.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    data.put_list("PlayerScores", vec![NbtTag::Compound(score)]);
    data.put_compound("DisplaySlots", NbtCompound::new());
    data.put_list("Teams", vec![NbtTag::Compound(team)]);
    data
}

#[test]
fn scoreboard_omitted_fields_use_vanilla_defaults_without_expanding_records()
-> Result<(), Box<dyn Error>> {
    let data = minimal_payload();
    let board = Scoreboard::from_save_data(&data)?;
    let objective = board.get_objective("points").ok_or("missing objective")?;
    assert_eq!(objective.criterion, "dummy");
    assert_eq!(objective.render_type, RenderType::Integer);
    assert!(!objective.display_auto_update);
    let score = board.get_score("holder", "points").ok_or("missing score")?;
    assert_eq!(score.value.0, 0);
    assert!(!score.locked);
    let team = board.get_team("red").ok_or("missing team")?;
    assert_eq!(team.options, 3);
    assert_eq!(team.color, None);
    assert_eq!(team.nametag_visibility, NameTagVisibility::Always);
    assert_eq!(team.deathmessage_visibility, NameTagVisibility::Always);
    assert_eq!(team.collision_rule, CollisionRule::Always);
    assert_eq!(team.display_name.clone().get_text(), "red");
    assert!(team.player_prefix.clone().get_text().is_empty());
    assert!(team.player_suffix.clone().get_text().is_empty());
    assert_eq!(board.to_save_data(), data);
    Ok(())
}

#[test]
fn scoreboard_score_value_update_retains_style_until_explicit_format_replacement()
-> Result<(), Box<dyn Error>> {
    let mut data = minimal_payload();
    let mut contents = NbtCompound::new();
    contents.put_string("minecraft:custom_name", "Treasured diamond".into());
    let mut hover = NbtCompound::new();
    hover.put_string("action", "show_item".into());
    hover.put_string("id", "minecraft:diamond".into());
    hover.put_compound("components", contents);
    let mut format = NbtCompound::new();
    format.put_string("type", "minecraft:styled".into());
    format.put_string("color", "red".into());
    // Style.Serializer.MAP_CODEC stores item components that Pumpkin's HoverEvent omits.
    format.put_compound("hover_event", hover);
    let mut score = NbtCompound::new();
    score.put_string("Name", "holder".into());
    score.put_string("Objective", "points".into());
    score.put_compound("format", format.clone());
    data.put_list("PlayerScores", vec![NbtTag::Compound(score)]);
    let mut board = Scoreboard::from_save_data(&data)?;
    board.set_score_value(&NoTarget, "holder", "points", 23);
    let output = board.to_save_data();
    let saved = output.get_list("PlayerScores").ok_or("missing scores")?[0]
        .extract_compound()
        .ok_or("invalid score")?;
    assert_eq!(saved.get_int("Score"), Some(23));
    assert_eq!(saved.get_compound("format"), Some(&format));
    assert!(
        !board
            .get_score("holder", "points")
            .ok_or("missing score")?
            .locked
    );
    let projected = board
        .get_score("holder", "points")
        .ok_or("missing score")?
        .number_format
        .clone();
    board.set_score_number_format(&NoTarget, "holder", "points", projected);
    let output = board.to_save_data();
    let saved = output.get_list("PlayerScores").ok_or("missing scores")?[0]
        .extract_compound()
        .ok_or("invalid score")?;
    let mut replacement = NbtCompound::new();
    replacement.put_string("type", "minecraft:styled".into());
    replacement.put_string("color", "red".into());
    assert_eq!(saved.get_compound("format"), Some(&replacement));
    board.set_score_number_format(&NoTarget, "holder", "points", None);
    let output = board.to_save_data();
    let saved = output.get_list("PlayerScores").ok_or("missing scores")?[0]
        .extract_compound()
        .ok_or("invalid score")?;
    assert!(saved.get("format").is_none());
    Ok(())
}

#[test]
fn scoreboard_orphan_records_survive_until_explicit_removal() -> Result<(), Box<dyn Error>> {
    let mut data = minimal_payload();
    let mut score = NbtCompound::new();
    score.put_string("Name", "orphan".into());
    score.put_string("Objective", "missing".into());
    let mut slots = NbtCompound::new();
    slots.put_string("sidebar", "missing".into());
    data.put_list("PlayerScores", vec![NbtTag::Compound(score.clone())]);
    data.put_compound("DisplaySlots", slots.clone());
    let mut board = Scoreboard::from_save_data(&data)?;
    assert!(board.get_score("orphan", "missing").is_none());
    assert!(
        board
            .get_display_objective(ScoreboardDisplaySlot::Sidebar)
            .is_none()
    );
    board.set_score_value(&NoTarget, "holder", "points", 8);
    let saved = board.to_save_data();
    assert!(
        saved
            .get_list("PlayerScores")
            .ok_or("missing scores")?
            .contains(&NbtTag::Compound(score))
    );
    assert_eq!(saved.get_compound("DisplaySlots"), Some(&slots));
    board.clear_display_objective(&NoTarget, ScoreboardDisplaySlot::Sidebar);
    board.reset_scores_for_entity(&NoTarget, "orphan");
    let saved = board.to_save_data();
    assert!(
        saved
            .get_compound("DisplaySlots")
            .ok_or("missing slots")?
            .is_empty()
    );
    assert_eq!(
        saved
            .get_list("PlayerScores")
            .ok_or("missing scores")?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn scoreboard_invalid_types_and_duplicate_orphan_scores_are_rejected() {
    let mut wrong_type = minimal_payload();
    wrong_type.put_string("Teams", "not a list".into());
    assert!(Scoreboard::from_save_data(&wrong_type).is_err());
    let mut orphan = NbtCompound::new();
    orphan.put_string("Name", "orphan".into());
    orphan.put_string("Objective", "missing".into());
    let mut duplicate = minimal_payload();
    duplicate.put_list(
        "PlayerScores",
        vec![NbtTag::Compound(orphan.clone()), NbtTag::Compound(orphan)],
    );
    assert!(Scoreboard::from_save_data(&duplicate).is_err());
}

#[test]
fn scoreboard_saved_snapshot_cannot_clear_a_later_mutation() -> Result<(), Box<dyn Error>> {
    let mut board = Scoreboard::from_save_data(&minimal_payload())?;
    board.set_score_value(&NoTarget, "holder", "points", 1);
    let snapshot_revision = board.revision();
    board.set_score_value(&NoTarget, "holder", "points", 2);
    board.mark_saved(snapshot_revision);
    assert!(board.is_dirty());
    board.mark_saved(board.revision());
    assert!(!board.is_dirty());
    board.get_team_mut("red").ok_or("missing team")?.options = 1;
    assert!(board.is_dirty());
    let revision = board.revision();
    board.mark_saved(revision);
    board
        .get_objective_mut("points")
        .ok_or("missing objective")?
        .display_auto_update = true;
    assert!(board.is_dirty());
    assert!(board.revision() > revision);
    Ok(())
}
