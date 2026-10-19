use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{NumberFormat, codec::var_int::VarInt, java::client::play::RenderType};
use pumpkin_util::text::{TextComponent, color::NamedColor, component_codec, style::Style};
use pumpkin_world::world_info::WorldInfoError;

use super::{CollisionRule, NameTagVisibility, ScoreboardObjective, ScoreboardScore, Team};

pub const DISPLAY_SLOTS: [(ScoreboardDisplaySlot, &str); 19] = [
    (ScoreboardDisplaySlot::List, "list"),
    (ScoreboardDisplaySlot::Sidebar, "sidebar"),
    (ScoreboardDisplaySlot::BelowName, "below_name"),
    (ScoreboardDisplaySlot::TeamBlack, "sidebar.team.black"),
    (
        ScoreboardDisplaySlot::TeamDarkBlue,
        "sidebar.team.dark_blue",
    ),
    (
        ScoreboardDisplaySlot::TeamDarkGreen,
        "sidebar.team.dark_green",
    ),
    (
        ScoreboardDisplaySlot::TeamDarkAqua,
        "sidebar.team.dark_aqua",
    ),
    (ScoreboardDisplaySlot::TeamDarkRed, "sidebar.team.dark_red"),
    (
        ScoreboardDisplaySlot::TeamDarkPurple,
        "sidebar.team.dark_purple",
    ),
    (ScoreboardDisplaySlot::TeamGold, "sidebar.team.gold"),
    (ScoreboardDisplaySlot::TeamGray, "sidebar.team.gray"),
    (
        ScoreboardDisplaySlot::TeamDarkGray,
        "sidebar.team.dark_gray",
    ),
    (ScoreboardDisplaySlot::TeamBlue, "sidebar.team.blue"),
    (ScoreboardDisplaySlot::TeamGreen, "sidebar.team.green"),
    (ScoreboardDisplaySlot::TeamAqua, "sidebar.team.aqua"),
    (ScoreboardDisplaySlot::TeamRed, "sidebar.team.red"),
    (
        ScoreboardDisplaySlot::TeamLightPurple,
        "sidebar.team.light_purple",
    ),
    (ScoreboardDisplaySlot::TeamYellow, "sidebar.team.yellow"),
    (ScoreboardDisplaySlot::TeamWhite, "sidebar.team.white"),
];

pub(super) fn invalid(reason: &str) -> WorldInfoError {
    WorldInfoError::DeserializationError(format!("Invalid scoreboard data: {reason}"))
}

pub(super) fn compound<'a>(
    tag: &'a NbtTag,
    field: &str,
) -> Result<&'a NbtCompound, WorldInfoError> {
    tag.extract_compound()
        .ok_or_else(|| invalid(&format!("{field} must be a compound")))
}

pub(super) fn list<'a>(data: &'a NbtCompound, field: &str) -> Result<&'a [NbtTag], WorldInfoError> {
    data.get(field).map_or(Ok(&[]), |tag| {
        tag.extract_list()
            .ok_or_else(|| invalid(&format!("{field} must be a list")))
    })
}

fn required_string(data: &NbtCompound, field: &str) -> Result<String, WorldInfoError> {
    data.get_string(field)
        .map(str::to_string)
        .ok_or_else(|| invalid(&format!("{field} must be a string")))
}

fn string<'a>(
    data: &'a NbtCompound,
    field: &str,
    default: &'a str,
) -> Result<&'a str, WorldInfoError> {
    data.get(field).map_or(Ok(default), |tag| {
        tag.extract_string()
            .ok_or_else(|| invalid(&format!("{field} must be a string")))
    })
}

fn boolean(data: &NbtCompound, field: &str, default: bool) -> Result<bool, WorldInfoError> {
    data.get(field).map_or(Ok(default), |tag| {
        tag.extract_bool()
            .ok_or_else(|| invalid(&format!("{field} must be a boolean")))
    })
}

fn component(data: &NbtCompound, field: &str) -> Result<Option<TextComponent>, WorldInfoError> {
    data.get(field)
        .map(|tag| {
            component_codec::read_component(tag)
                .ok_or_else(|| invalid(&format!("{field} must be a component")))
        })
        .transpose()
}

pub(super) fn read_objective(data: &NbtCompound) -> Result<ScoreboardObjective, WorldInfoError> {
    Ok(ScoreboardObjective {
        name: required_string(data, "Name")?,
        criterion: string(data, "CriteriaName", "dummy")?.to_string(),
        display_name: component(data, "DisplayName")?
            .ok_or_else(|| invalid("objective DisplayName is required"))?,
        render_type: match string(data, "RenderType", "integer")? {
            "integer" => RenderType::Integer,
            "hearts" => RenderType::Hearts,
            _ => return Err(invalid("unknown render type")),
        },
        display_auto_update: boolean(data, "display_auto_update", false)?,
        number_format: read_format(data)?,
    })
}

pub(super) fn write_objective(value: &ScoreboardObjective) -> NbtCompound {
    let mut data = NbtCompound::new();
    data.put_string("Name", value.name.clone());
    data.put_string("CriteriaName", value.criterion.clone());
    data.put(
        "DisplayName",
        component_codec::write_component(&value.display_name),
    );
    data.put_string(
        "RenderType",
        match value.render_type {
            RenderType::Integer => "integer",
            RenderType::Hearts => "hearts",
        }
        .to_string(),
    );
    data.put_bool("display_auto_update", value.display_auto_update);
    write_format(&mut data, value.number_format.as_ref());
    data
}

pub(super) fn read_score(data: &NbtCompound) -> Result<ScoreboardScore, WorldInfoError> {
    Ok(ScoreboardScore {
        entity_name: required_string(data, "Name")?,
        objective_name: required_string(data, "Objective")?,
        value: VarInt(data.get("Score").map_or(Ok(0), |tag| {
            tag.extract_int()
                .ok_or_else(|| invalid("Score must be an integer"))
        })?),
        display_name: component(data, "display")?,
        number_format: read_format(data)?,
        // Score.Packed differs from a newly constructed Score: missing Locked is false.
        locked: boolean(data, "Locked", false)?,
    })
}

pub(super) fn write_score(value: &ScoreboardScore) -> NbtCompound {
    let mut data = NbtCompound::new();
    data.put_string("Name", value.entity_name.clone());
    data.put_string("Objective", value.objective_name.clone());
    data.put_int("Score", value.value.0);
    data.put_bool("Locked", value.locked);
    if let Some(display) = &value.display_name {
        data.put("display", component_codec::write_component(display));
    }
    write_format(&mut data, value.number_format.as_ref());
    data
}

fn visibility(data: &NbtCompound, field: &str) -> Result<NameTagVisibility, WorldInfoError> {
    match string(data, field, "always")? {
        "always" => Ok(NameTagVisibility::Always),
        "never" => Ok(NameTagVisibility::Never),
        "hideForOtherTeams" => Ok(NameTagVisibility::HideForOtherTeams),
        "hideForOwnTeam" => Ok(NameTagVisibility::HideForOwnTeam),
        _ => Err(invalid("unknown team visibility")),
    }
}

pub(super) fn read_team(data: &NbtCompound) -> Result<Team, WorldInfoError> {
    let name = required_string(data, "Name")?;
    let color = match data.get("TeamColor") {
        None => None,
        Some(tag) => Some(
            tag.extract_string()
                .and_then(|name| NamedColor::try_from(name).ok())
                .ok_or_else(|| invalid("unknown team color"))?,
        ),
    };
    Ok(Team {
        display_name: component(data, "DisplayName")?
            .unwrap_or_else(|| TextComponent::text(name.clone())),
        name,
        options: i8::from(boolean(data, "AllowFriendlyFire", true)?)
            | (i8::from(boolean(data, "SeeFriendlyInvisibles", true)?) << 1),
        nametag_visibility: visibility(data, "NameTagVisibility")?,
        deathmessage_visibility: visibility(data, "DeathMessageVisibility")?,
        collision_rule: match string(data, "CollisionRule", "always")? {
            "always" => CollisionRule::Always,
            "never" => CollisionRule::Never,
            "pushOtherTeams" => CollisionRule::PushOtherTeams,
            "pushOwnTeam" => CollisionRule::PushOwnTeam,
            _ => return Err(invalid("unknown collision rule")),
        },
        color,
        player_prefix: component(data, "MemberNamePrefix")?.unwrap_or_else(TextComponent::empty),
        player_suffix: component(data, "MemberNameSuffix")?.unwrap_or_else(TextComponent::empty),
        players: list(data, "Players")?
            .iter()
            .map(|tag| {
                tag.extract_string()
                    .map(str::to_string)
                    .ok_or_else(|| invalid("team member must be a string"))
            })
            .collect::<Result<_, _>>()?,
    })
}

pub(super) fn write_team(value: &Team) -> NbtCompound {
    let mut data = NbtCompound::new();
    data.put_string("Name", value.name.clone());
    data.put(
        "DisplayName",
        component_codec::write_component(&value.display_name),
    );
    if let Some(color) = value.color {
        data.put_string("TeamColor", color.to_string());
    }
    data.put_bool("AllowFriendlyFire", value.options & 1 != 0);
    data.put_bool("SeeFriendlyInvisibles", value.options & 2 != 0);
    data.put(
        "MemberNamePrefix",
        component_codec::write_component(&value.player_prefix),
    );
    data.put(
        "MemberNameSuffix",
        component_codec::write_component(&value.player_suffix),
    );
    data.put_string(
        "NameTagVisibility",
        value.nametag_visibility.to_str().to_string(),
    );
    data.put_string(
        "DeathMessageVisibility",
        value.deathmessage_visibility.to_str().to_string(),
    );
    data.put_string("CollisionRule", value.collision_rule.to_str().to_string());
    data.put_list(
        "Players",
        value
            .players
            .iter()
            .map(|name| NbtTag::String(name.clone().into()))
            .collect(),
    );
    data
}

fn read_format(data: &NbtCompound) -> Result<Option<NumberFormat>, WorldInfoError> {
    let Some(tag) = data.get("format") else {
        return Ok(None);
    };
    let format = compound(tag, "format")?;
    let kind = required_string(format, "type")?;
    match kind.strip_prefix("minecraft:").unwrap_or(&kind) {
        "blank" => Ok(Some(NumberFormat::Blank)),
        "fixed" => Ok(Some(NumberFormat::Fixed(
            component(format, "value")?.ok_or_else(|| invalid("fixed format value is required"))?,
        ))),
        "styled" => {
            // The sidecar retains the complete Style.Serializer.MAP_CODEC record.
            let json = component_codec::canonical_component(tag);
            let style = serde_json::from_value::<Style>(json).unwrap_or_default();
            Ok(Some(NumberFormat::Styled(style)))
        }
        _ => Err(invalid("unknown number format")),
    }
}

fn write_format(data: &mut NbtCompound, format: Option<&NumberFormat>) {
    let Some(format) = format else {
        return;
    };
    let mut tag = NbtCompound::new();
    let kind = match format {
        NumberFormat::Blank => "minecraft:blank",
        NumberFormat::Fixed(component) => {
            tag.put("value", component_codec::write_component(component));
            "minecraft:fixed"
        }
        NumberFormat::Styled(style) => {
            let mut component = TextComponent::empty();
            *component.0.style = style.clone();
            if let NbtTag::Compound(mut fields) = component_codec::write_component(&component) {
                fields.child_tags.remove("text");
                tag = fields;
            }
            "minecraft:styled"
        }
    };
    tag.put_string("type", kind.to_string());
    data.put_compound("format", tag);
}
