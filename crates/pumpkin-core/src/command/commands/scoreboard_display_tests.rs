use std::sync::Arc;

use super::{DisplaySlotArgument, StyleArgument};
use crate::command::{
    CommandSource, argument_types::argument_type::ArgumentType,
    node::dispatcher::CommandDispatcher, string_reader::StringReader,
};
use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_util::{permission::PermissionRegistry, text::style::Style};

#[test]
fn scoreboard_display_command_tree_accepts_vanilla_format_and_slot_syntax() {
    let mut dispatcher = CommandDispatcher::new();
    super::super::register(&mut dispatcher, &PermissionRegistry::default());
    let source = Arc::new(CommandSource::dummy());
    for input in [
        "scoreboard objectives setdisplay sidebar.team.red tally",
        "scoreboard objectives setdisplay below_name",
        "scoreboard objectives modify tally displayautoupdate true",
        "scoreboard objectives modify tally rendertype hearts",
        "scoreboard objectives modify tally displayname {\"text\":\"Tally\",\"bold\":true}",
        "scoreboard objectives modify tally numberformat",
        "scoreboard objectives modify tally numberformat blank",
        "scoreboard objectives modify tally numberformat fixed {\"text\":\"Many\"}",
        "scoreboard objectives modify tally numberformat styled {color:'red',bold:true}",
        "scoreboard players display name Holder tally",
        "scoreboard players display name Holder tally {\"text\":\"Alias\"}",
        "scoreboard players display numberformat Holder tally",
        "scoreboard players display numberformat Holder tally blank",
        "scoreboard players display numberformat Holder tally fixed {\"text\":\"Many\"}",
        "scoreboard players display numberformat Holder tally styled {color:'red'}",
    ] {
        let parsed = dispatcher.parse_input(input, &source);
        assert!(parsed.errors.is_empty(), "{input}: {:?}", parsed.errors);
        assert_eq!(parsed.reader.remaining_length(), 0, "{input}");
    }
}

#[test]
fn scoreboard_display_arguments_reject_invalid_slots_and_styles() {
    let slot = DisplaySlotArgument;
    assert_eq!(
        slot.parse(&mut StringReader::new("below_name")).unwrap(),
        ScoreboardDisplaySlot::BelowName
    );
    assert!(slot.parse(&mut StringReader::new("belowName")).is_err());
    assert!(
        slot.parse(&mut StringReader::new("sidebar.team.not_a_color"))
            .is_err()
    );
    let style = StyleArgument
        .parse(&mut StringReader::new("{bold:true}"))
        .unwrap();
    assert_eq!(style, Style::default().bold());
    assert!(
        StyleArgument
            .parse(&mut StringReader::new("{bold:'invalid'}"))
            .is_err()
    );
    assert!(
        StyleArgument
            .parse(&mut StringReader::new("'red'"))
            .is_err()
    );
}
