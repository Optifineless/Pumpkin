use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use super::{NoTarget, Scoreboard};

#[derive(Clone, Copy, Debug)]
enum ComponentField {
    ObjectiveDisplayName,
    TeamDisplayName,
    TeamPrefix,
    TeamSuffix,
    ScoreDisplay,
    ObjectiveFixedFormat,
    ScoreFixedFormat,
}

const COMPONENT_FIELDS: [ComponentField; 7] = [
    ComponentField::ObjectiveDisplayName,
    ComponentField::TeamDisplayName,
    ComponentField::TeamPrefix,
    ComponentField::TeamSuffix,
    ComponentField::ScoreDisplay,
    ComponentField::ObjectiveFixedFormat,
    ComponentField::ScoreFixedFormat,
];

fn fixed_format(value: NbtTag) -> NbtCompound {
    let mut format = NbtCompound::new();
    format.put_string("type", "minecraft:fixed".into());
    format.put("value", value);
    format
}

fn payload(field: ComponentField, value: NbtTag, orphan_score: bool) -> NbtCompound {
    let mut objective = NbtCompound::new();
    objective.put_string("Name", "points".into());
    objective.put_string("DisplayName", "Points".into());
    let mut score = NbtCompound::new();
    score.put_string("Name", "holder".into());
    score.put_string(
        "Objective",
        if orphan_score { "missing" } else { "points" }.into(),
    );
    let mut team = NbtCompound::new();
    team.put_string("Name", "red".into());
    match field {
        ComponentField::ObjectiveDisplayName => objective.put("DisplayName", value),
        ComponentField::TeamDisplayName => team.put("DisplayName", value),
        ComponentField::TeamPrefix => team.put("MemberNamePrefix", value),
        ComponentField::TeamSuffix => team.put("MemberNameSuffix", value),
        ComponentField::ScoreDisplay => score.put("display", value),
        ComponentField::ObjectiveFixedFormat => {
            objective.put_compound("format", fixed_format(value));
        }
        ComponentField::ScoreFixedFormat => score.put_compound("format", fixed_format(value)),
    }
    let mut data = NbtCompound::new();
    data.put_list("Objectives", vec![NbtTag::Compound(objective)]);
    data.put_list("PlayerScores", vec![NbtTag::Compound(score)]);
    data.put_list("Teams", vec![NbtTag::Compound(team)]);
    data.put_compound("DisplaySlots", NbtCompound::new());
    data
}

fn unsupported_component() -> NbtCompound {
    let mut content = NbtCompound::new();
    content.put_string("nbt", "scores[0].name".into());
    content.put_string("storage", "minecraft:scoreboard".into());
    content.put_string("future_style", "retained verbatim".into());
    let mut future_style = NbtCompound::new();
    future_style.put_long("future_number", 9_007_199_254_740_993);
    content.put_compound("future_style_data", future_style);
    content
}

#[test]
fn scoreboard_saved_components_reject_malformed_content_at_every_field() {
    for field in COMPONENT_FIELDS {
        for malformed in [
            NbtTag::Compound(NbtCompound::new()),
            NbtTag::List(Vec::new()),
            NbtTag::Int(42),
        ] {
            assert!(
                Scoreboard::from_save_data(&payload(field, malformed.clone(), false)).is_err(),
                "{field:?} admitted malformed saved component {malformed:?}"
            );
        }
    }
}

#[test]
fn scoreboard_saved_components_reject_invalid_extra_and_translation_children() {
    let mut extra = NbtCompound::new();
    extra.put_string("text", "parent".into());
    extra.put_list("extra", vec![NbtTag::Compound(NbtCompound::new())]);
    let mut translation = NbtCompound::new();
    translation.put_string("translate", "chat.type.text".into());
    translation.put_list("with", vec![NbtTag::Compound(NbtCompound::new())]);
    let mut empty_extra = NbtCompound::new();
    empty_extra.put_string("text", "parent".into());
    empty_extra.put_list("extra", Vec::new());
    for field in COMPONENT_FIELDS {
        for malformed in [&extra, &translation, &empty_extra] {
            assert!(
                Scoreboard::from_save_data(&payload(
                    field,
                    NbtTag::Compound(malformed.clone()),
                    false,
                ))
                .is_err(),
                "{field:?} admitted malformed saved component child {malformed:?}"
            );
        }
    }
}

#[test]
fn scoreboard_saved_components_validate_orphan_score_display_and_fixed_format() {
    for field in [
        ComponentField::ScoreDisplay,
        ComponentField::ScoreFixedFormat,
    ] {
        assert!(
            Scoreboard::from_save_data(&payload(field, NbtTag::Compound(NbtCompound::new()), true))
                .is_err(),
            "{field:?} admitted malformed component from an orphan score"
        );
    }
}

#[test]
fn scoreboard_saved_components_validate_selector_object_and_hover_children() {
    let malformed = NbtTag::Compound(NbtCompound::new());
    let mut selector = NbtCompound::new();
    selector.put_string("selector", "@a".into());
    selector.put("separator", malformed.clone());
    let mut object = NbtCompound::new();
    object.put_string("sprite", "minecraft:block/stone".into());
    object.put("fallback", malformed.clone());
    let mut show_text = NbtCompound::new();
    show_text.put_string("action", "show_text".into());
    show_text.put("value", malformed.clone());
    let mut show_entity = NbtCompound::new();
    show_entity.put_string("action", "show_entity".into());
    show_entity.put_string("id", "minecraft:pig".into());
    show_entity.put_string("uuid", "00000000-0000-0000-0000-000000000001".into());
    show_entity.put("name", malformed);
    let children = [show_text, show_entity].map(|hover| {
        let mut parent = NbtCompound::new();
        parent.put_string("text", "hover parent".into());
        parent.put_compound("hover_event", hover);
        parent
    });
    for field in COMPONENT_FIELDS {
        for parent in [&selector, &object, &children[0], &children[1]] {
            assert!(
                Scoreboard::from_save_data(&payload(
                    field,
                    NbtTag::Compound(parent.clone()),
                    false,
                ))
                .is_err(),
                "{field:?} admitted malformed nested component {parent:?}"
            );
        }
    }
}

#[test]
fn scoreboard_saved_components_validate_hover_children_in_styled_number_formats() {
    for list in ["Objectives", "PlayerScores"] {
        let mut hover = NbtCompound::new();
        hover.put_string("action", "show_text".into());
        hover.put_compound("value", NbtCompound::new());
        let mut format = NbtCompound::new();
        format.put_string("type", "minecraft:styled".into());
        format.put_compound("hover_event", hover);
        let mut data = payload(
            ComponentField::ObjectiveDisplayName,
            NbtTag::String("Points".into()),
            false,
        );
        let mut record = data.get_list(list).unwrap()[0]
            .extract_compound()
            .unwrap()
            .clone();
        record.put_compound("format", format);
        data.put_list(list, vec![NbtTag::Compound(record)]);
        assert!(
            Scoreboard::from_save_data(&data).is_err(),
            "{list} admitted malformed component through styled-format hover value"
        );
    }
}

#[test]
fn scoreboard_saved_components_preserve_vanilla_lenient_nbt_separator() {
    let mut nbt = unsupported_component();
    // NbtContents.MAP_CODEC uses lenientOptionalFieldOf for its separator.
    nbt.put_compound("separator", NbtCompound::new());
    for field in COMPONENT_FIELDS {
        let data = payload(field, NbtTag::Compound(nbt.clone()), false);
        let board = Scoreboard::from_save_data(&data).unwrap_or_else(|error| {
            panic!("{field:?} rejected a lenient optional NBT separator: {error}")
        });
        assert_eq!(board.to_save_data(), data);
    }
}

#[test]
fn scoreboard_saved_components_preserve_player_object_with_empty_property_arrays() {
    for values in [
        NbtTag::ByteArray(Vec::new().into()),
        NbtTag::IntArray(Vec::new()),
        NbtTag::LongArray(Vec::new()),
    ] {
        let mut properties = NbtCompound::new();
        properties.put("textures", values);
        let mut profile = NbtCompound::new();
        profile.put_string("name", "Holder".into());
        profile.put_compound("properties", properties);
        let mut object = NbtCompound::new();
        object.put_compound("player", profile);
        object.put_string("future_style", "keep object style".into());
        for field in COMPONENT_FIELDS {
            let data = payload(field, NbtTag::Compound(object.clone()), false);
            let board = Scoreboard::from_save_data(&data).unwrap_or_else(|error| {
                panic!("{field:?} rejected a valid player object: {error}")
            });
            assert_eq!(board.to_save_data(), data);
        }
    }
}

#[test]
fn scoreboard_saved_components_preserve_unsupported_contents_and_unknown_styles() {
    for field in COMPONENT_FIELDS {
        let data = payload(field, NbtTag::Compound(unsupported_component()), false);
        let mut board = Scoreboard::from_save_data(&data).unwrap_or_else(|error| {
            panic!("{field:?} rejected valid unsupported content: {error}")
        });
        board.set_score_value(&NoTarget, "temporary", "points", 1);
        board.reset_scores_for_entity(&NoTarget, "temporary");
        assert!(board.is_dirty());
        assert_eq!(
            board.to_save_data(),
            data,
            "{field:?} changed opaque content or unknown style fields"
        );
    }
}
