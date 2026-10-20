use super::*;
use crate::world::scoreboard::{CollisionRule, NameTagVisibility};
use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_protocol::{NumberFormat, java::client::play::RenderType};
use pumpkin_util::text::{
    TextComponent,
    color::{Color, NamedColor},
    style::Style,
};

fn assert_blue_format(format: Option<&NumberFormat>) {
    let Some(NumberFormat::Styled(style)) = format else {
        panic!("Reloaded score lost its styled number format");
    };
    let mut style = style.clone();
    let color = match style.color.take().unwrap() {
        Color::Rgb(rgb) => rgb,
        Color::Named(named) => named.to_rgb(),
        Color::Reset => panic!("Reloaded score lost its blue colour"),
    };
    assert_eq!(color, NamedColor::Blue.to_rgb());
    assert_eq!(style, Style::default().bold());
}

pub(super) fn assert_command_fields(world: &World) {
    let scoreboard = world.scoreboard.lock().unwrap();
    assert_eq!(scoreboard.get_objectives().len(), 2);
    let objective = scoreboard.get_objective("tally").unwrap();
    assert_eq!(objective.name, "tally");
    assert_eq!(objective.criterion, "dummy");
    assert_eq!(
        objective.display_name,
        TextComponent::text("Total").color_named(NamedColor::Gold)
    );
    assert_eq!(objective.render_type, RenderType::Hearts);
    assert!(objective.display_auto_update);
    assert_eq!(
        objective.number_format,
        Some(NumberFormat::Fixed(TextComponent::text("many")))
    );
    let score = scoreboard.get_score("Holder", "tally").unwrap();
    assert_eq!(score.entity_name, "Holder");
    assert_eq!(score.objective_name, "tally");
    assert_eq!(score.value.0, 37);
    assert_eq!(
        score.display_name,
        Some(TextComponent::text("Alias").italic())
    );
    assert_blue_format(score.number_format.as_ref());
    assert!(score.locked);
    let trigger = scoreboard
        .get_score("combat-test", "trigger_value")
        .unwrap();
    assert_eq!(trigger.value.0, 10);
    assert!(trigger.locked);
    assert_eq!(scoreboard.get_display_slots().len(), 1);
    assert_eq!(
        scoreboard.get_display_objective(ScoreboardDisplaySlot::TeamRed),
        Some("tally")
    );
    assert_eq!(scoreboard.get_teams().len(), 1);
    let team = scoreboard.get_team("red").unwrap();
    assert_eq!(team.name, "red");
    assert_eq!(team.display_name, TextComponent::text("Crimson"));
    assert_eq!(team.color, Some(NamedColor::Red));
    assert_eq!(team.options, 0);
    assert_eq!(
        team.nametag_visibility,
        NameTagVisibility::HideForOtherTeams
    );
    assert_eq!(
        team.deathmessage_visibility,
        NameTagVisibility::HideForOwnTeam
    );
    assert_eq!(team.collision_rule, CollisionRule::PushOwnTeam);
    assert_eq!(team.player_prefix, TextComponent::text("Prefix"));
    assert_eq!(team.player_suffix, TextComponent::text("Suffix"));
    assert_eq!(team.players, ["combat-test"]);
}
