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

// These are file payloads from the review, not values built by the display projector.
pub fn examples() -> Vec<NbtTag> {
    [
        serde_json::json!({"text":"x","color":"not_a_color"}),
        serde_json::json!({"text":"x","click_event":{"action":"open_url","url":"relative"}}),
        serde_json::json!({"text":"x","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":"invalid"}}),
        serde_json::json!({"selector":"@"}),
        serde_json::json!({"nbt":"[","storage":"minecraft:test"}),
        serde_json::json!({"nbt":"foo","source":"block","block":"0 0"}),
        serde_json::json!({"nbt":"foo","source":"entity","entity":"@"}),
        serde_json::json!({"text":"x","hover_event":{"action":"show_item","id":"minecraft:stone","components":{"minecraft:custom_name":{}}}}),
        serde_json::json!({"text":"x","click_event":{"action":"show_dialog","dialog":{"type":"minecraft:notice","title":{}}}}),
        serde_json::json!({"text":"x","hover_event":{"action":"show_entity","id":"minecraft:no_such_entity","uuid":"00000000-0000-0000-0000-000000000001"}}),
        serde_json::json!({"text":"x","color":"#€123"}),
        serde_json::json!({"text":"x","color":"#１123"}),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!({"text":"parent","extra":[{}]}),
        serde_json::json!({"translate":"unknown","with":[{}],"fallback":"fallback"}),
        serde_json::json!({"text":"x","font":"..:invalid","bold":"invalid","italic":{},"shadow_color":"bad","insertion":"drop"}),
        serde_json::json!({"text":"safe","font":"minecraft:default","bold":true,"italic":false,"underlined":true,"strikethrough":false,"obfuscated":false,"extra":[{"translate":"custom.nested","fallback":"%s %s","with":["plain",{"text":"argument","color":"#12ABCD"}]}]}),
    ].into_iter().map(fixture_tag).collect()
}

fn fixture_tag(value: serde_json::Value) -> NbtTag {
    match value {
        serde_json::Value::String(value) => NbtTag::String(value.into()),
        serde_json::Value::Object(fields) => {
            let mut compound = NbtCompound::new();
            for (name, value) in fields {
                compound.put(&name, fixture_tag(value));
            }
            NbtTag::Compound(compound)
        }
        serde_json::Value::Array(values) => {
            NbtTag::List(values.into_iter().map(fixture_tag).collect())
        }
        serde_json::Value::Bool(value) => NbtTag::Byte(i8::from(value)),
        _ => panic!("Review fixtures only use strings, booleans, lists and compounds"),
    }
}

#[test]
fn scoreboard_saved_components_reject_only_wrong_outer_types_at_every_field() {
    for field in COMPONENT_FIELDS {
        assert!(Scoreboard::from_save_data(&payload(field, NbtTag::Int(42), false)).is_err());
    }
}

#[test]
fn scoreboard_saved_components_project_and_preserve_every_review_example() {
    for field in COMPONENT_FIELDS {
        for example in examples() {
            let data = payload(field, example.clone(), false);
            let mut board = Scoreboard::from_save_data(&data)
                .unwrap_or_else(|error| panic!("{field:?} rejected {example:?}: {error}"));
            board.set_score_value(&NoTarget, "temporary", "points", 1);
            board.reset_scores_for_entity(&NoTarget, "temporary");
            assert!(board.is_dirty());
            assert_eq!(
                board.to_save_data(),
                data,
                "{field:?} lost raw NBT {example:?}"
            );
        }
    }
    for field in [
        ComponentField::ScoreDisplay,
        ComponentField::ScoreFixedFormat,
    ] {
        let data = payload(field, NbtTag::Compound(NbtCompound::new()), true);
        assert_eq!(
            Scoreboard::from_save_data(&data).unwrap().to_save_data(),
            data
        );
    }
}

#[test]
fn scoreboard_saved_components_project_styled_formats_and_preserve_raw_styles() {
    for list in ["Objectives", "PlayerScores"] {
        for example in examples()
            .into_iter()
            .filter_map(|tag| tag.extract_compound().cloned())
        {
            let mut format = example;
            format.put_string("type", "minecraft:styled".into());
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
            assert_eq!(
                Scoreboard::from_save_data(&data).unwrap().to_save_data(),
                data
            );
        }
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

#[test]
fn scoreboard_native_component_replacements_do_not_resurrect_raw_fields() {
    use pumpkin_protocol::NumberFormat;
    use pumpkin_util::text::TextComponent;
    let original = fixture_tag(serde_json::json!({
        "text":"A", "click_event":{"action":"run_command","command":"/old"}
    }));
    for field in COMPONENT_FIELDS {
        let mut board =
            Scoreboard::from_save_data(&payload(field, original.clone(), false)).unwrap();
        for text in ["B", "A"] {
            let text = TextComponent::text(text);
            match field {
                ComponentField::ObjectiveDisplayName | ComponentField::ObjectiveFixedFormat => {
                    let mut objective = board.get_objective("points").unwrap().clone();
                    if matches!(field, ComponentField::ObjectiveDisplayName) {
                        objective.display_name = text;
                    } else {
                        objective.number_format = Some(NumberFormat::Fixed(text));
                    }
                    board.update_objective(&NoTarget, objective);
                }
                ComponentField::ScoreDisplay | ComponentField::ScoreFixedFormat => {
                    let mut score = board.get_score("holder", "points").unwrap().clone();
                    if matches!(field, ComponentField::ScoreDisplay) {
                        score.display_name = Some(text);
                    } else {
                        score.number_format = Some(NumberFormat::Fixed(text));
                    }
                    board.update_score(&NoTarget, score);
                }
                ComponentField::TeamDisplayName
                | ComponentField::TeamPrefix
                | ComponentField::TeamSuffix => {
                    let mut team = board.get_team("red").unwrap().clone();
                    match field {
                        ComponentField::TeamDisplayName => team.display_name = text,
                        ComponentField::TeamPrefix => team.player_prefix = text,
                        _ => team.player_suffix = text,
                    }
                    board.update_team(&NoTarget, team);
                }
            }
        }
        assert_eq!(
            board.to_save_data(),
            payload(field, NbtTag::String("A".into()), false),
            "{field:?}"
        );
    }
}
