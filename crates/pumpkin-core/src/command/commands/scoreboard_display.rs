use super::{ARG_DISPLAY_NAME, ARG_OBJECTIVE, ARG_TARGETS, holder_component};
use crate::command::{
    CommandSource,
    argument_builder::{ArgumentBuilder, LiteralArgumentBuilder, argument, literal},
    argument_types::{
        argument_type::{ArgumentType, JavaClientArgumentType},
        component::ComponentArgumentType,
        core::bool::BoolArgumentType,
        objective::ObjectiveArgumentType,
        score_holder::ScoreHolderArgumentType,
    },
    context::command_context::CommandContext,
    errors::{command_syntax_error::CommandSyntaxError, error_types::CommandErrorType},
    node::{CommandExecutor, CommandExecutorResult},
    snbt::SnbtParser,
    string_reader::StringReader,
    suggestion::suggestions::{Suggestions, SuggestionsBuilder},
};
use crate::world::scoreboard::{DISPLAY_SLOTS, ScoreboardScore};
use pumpkin_data::{scoreboard::ScoreboardDisplaySlot, translation::java};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{NumberFormat, codec::var_int::VarInt, java::client::play::RenderType};
use pumpkin_util::text::{TextComponent, component_codec::canonical_component, style::Style};

#[cfg(test)]
#[path = "scoreboard_display_tests.rs"]
mod tests;

const ARG_SLOT: &str = "slot";
const ARG_VALUE: &str = "value";
const ARG_STYLE: &str = "style";
const INVALID_SLOT: CommandErrorType<1> = CommandErrorType::new(
    java::ARGUMENT_SCOREBOARDDISPLAYSLOT_INVALID,
    java::ARGUMENT_SCOREBOARDDISPLAYSLOT_INVALID,
);
const INVALID_STYLE: CommandErrorType<1> =
    CommandErrorType::new(java::ARGUMENT_STYLE_INVALID, java::ARGUMENT_STYLE_INVALID);
const ALREADY_EMPTY: CommandErrorType<0> = CommandErrorType::new(
    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_ALREADYEMPTY,
    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_ALREADYEMPTY,
);
const ALREADY_SET: CommandErrorType<0> = CommandErrorType::new(
    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_ALREADYSET,
    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_ALREADYSET,
);

struct DisplaySlotArgument;

impl ArgumentType<CommandSource> for DisplaySlotArgument {
    type Item = ScoreboardDisplaySlot;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let start = reader.cursor();
        let name = reader.read_unquoted_string();
        DISPLAY_SLOTS
            .iter()
            .find_map(|(slot, id)| (*id == name).then_some(*slot))
            .ok_or_else(|| {
                reader.set_cursor(start);
                INVALID_SLOT.create(reader, TextComponent::text(name))
            })
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::ScoreboardSlot
    }

    fn list_suggestions(
        &self,
        _context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> Suggestions {
        for (_, name) in DISPLAY_SLOTS {
            builder = builder.filter_and_suggest_one(name);
        }
        builder.build()
    }
}

struct StyleArgument;

impl ArgumentType<CommandSource> for StyleArgument {
    type Item = Style;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        // StyleArgument uses SnbtGrammar and Style.Serializer.CODEC in vanilla 26.3.
        let tag = SnbtParser::parse_for_commands(reader)?;
        if !matches!(tag, NbtTag::Compound(_)) {
            return Err(
                INVALID_STYLE.create(reader, TextComponent::text("Expected a style compound"))
            );
        }
        serde_json::from_value(canonical_component(&tag))
            .map_err(|error| INVALID_STYLE.create(reader, TextComponent::text(error.to_string())))
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::Style
    }
}

#[derive(Clone, Copy)]
enum FormatKind {
    Clear,
    Blank,
    Fixed,
    Styled,
}

impl FormatKind {
    fn get(self, context: &CommandContext) -> Result<Option<NumberFormat>, CommandSyntaxError> {
        Ok(match self {
            Self::Clear => None,
            Self::Blank => Some(NumberFormat::Blank),
            Self::Fixed => Some(NumberFormat::Fixed(ComponentArgumentType::get(
                context,
                ARG_DISPLAY_NAME,
            )?)),
            Self::Styled => Some(NumberFormat::Styled(
                context.get_argument::<Style>(ARG_STYLE)?.clone(),
            )),
        })
    }
}

#[derive(Clone, Copy)]
enum FormatTarget {
    Objective,
    Score,
}

fn add_number_formats<B: ArgumentBuilder>(builder: B, target: FormatTarget) -> B {
    // ScoreboardCommand.addNumberFormats also makes the bare node clear the override.
    builder
        .executes(FormatExecutor {
            target,
            kind: FormatKind::Clear,
        })
        .then(literal("blank").executes(FormatExecutor {
            target,
            kind: FormatKind::Blank,
        }))
        .then(
            literal("fixed").then(argument(ARG_DISPLAY_NAME, ComponentArgumentType).executes(
                FormatExecutor {
                    target,
                    kind: FormatKind::Fixed,
                },
            )),
        )
        .then(
            literal("styled").then(argument(ARG_STYLE, StyleArgument).executes(FormatExecutor {
                target,
                kind: FormatKind::Styled,
            })),
        )
}

pub(super) fn objectives_modify() -> LiteralArgumentBuilder {
    literal("modify").then(
        argument(ARG_OBJECTIVE, ObjectiveArgumentType)
            .then(
                literal("displayname").then(
                    argument(ARG_DISPLAY_NAME, ComponentArgumentType)
                        .executes(ObjectiveModifyExecutor(ObjectiveModification::DisplayName)),
                ),
            )
            .then(
                literal("rendertype")
                    .then(literal("integer").executes(ObjectiveModifyExecutor(
                        ObjectiveModification::RenderType(RenderType::Integer),
                    )))
                    .then(literal("hearts").executes(ObjectiveModifyExecutor(
                        ObjectiveModification::RenderType(RenderType::Hearts),
                    ))),
            )
            .then(literal("displayautoupdate").then(
                argument(ARG_VALUE, BoolArgumentType).executes(ObjectiveModifyExecutor(
                    ObjectiveModification::DisplayAutoUpdate,
                )),
            ))
            .then(add_number_formats(
                literal("numberformat"),
                FormatTarget::Objective,
            )),
    )
}

pub(super) fn objectives_setdisplay() -> LiteralArgumentBuilder {
    literal("setdisplay").then(
        argument(ARG_SLOT, DisplaySlotArgument)
            .executes(SetDisplayExecutor(false))
            .then(
                argument(ARG_OBJECTIVE, ObjectiveArgumentType).executes(SetDisplayExecutor(true)),
            ),
    )
}

pub(super) fn players_display() -> LiteralArgumentBuilder {
    literal("display")
        .then(
            literal("name").then(
                argument(ARG_TARGETS, ScoreHolderArgumentType::Multiple).then(
                    argument(ARG_OBJECTIVE, ObjectiveArgumentType)
                        .executes(ScoreDisplayExecutor(false))
                        .then(
                            argument(ARG_DISPLAY_NAME, ComponentArgumentType)
                                .executes(ScoreDisplayExecutor(true)),
                        ),
                ),
            ),
        )
        .then(literal("numberformat").then(
            argument(ARG_TARGETS, ScoreHolderArgumentType::Multiple).then(add_number_formats(
                argument(ARG_OBJECTIVE, ObjectiveArgumentType),
                FormatTarget::Score,
            )),
        ))
}

struct SetDisplayExecutor(bool);

impl CommandExecutor for SetDisplayExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let slot = *context.get_argument::<ScoreboardDisplaySlot>(ARG_SLOT)?;
        let objective_name = self
            .0
            .then(|| ObjectiveArgumentType::get(context, ARG_OBJECTIVE))
            .transpose()?;
        let world = context.world();
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let objective_display = objective_name
            .map(|name| {
                ObjectiveArgumentType::objective_or_error(&scoreboard, name)
                    .map(|objective| objective.display_name.clone())
            })
            .transpose()?;
        if scoreboard.get_display_objective(slot) == objective_name {
            return Err(if self.0 {
                ALREADY_SET.create_without_context()
            } else {
                ALREADY_EMPTY.create_without_context()
            });
        }
        scoreboard.set_display_objective(world, slot, objective_name);
        drop(scoreboard);
        let slot_name = DISPLAY_SLOTS
            .iter()
            .find_map(|(value, name)| (*value == slot).then_some(*name))
            .unwrap_or_default();
        let (key, arguments) = objective_display.map_or_else(
            || {
                (
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_CLEARED,
                    vec![TextComponent::text(slot_name)],
                )
            },
            |display| {
                (
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_DISPLAY_SET,
                    vec![TextComponent::text(slot_name), display],
                )
            },
        );
        context
            .source
            .send_feedback(TextComponent::translate_cross(key, key, arguments), true);
        Ok(0)
    }
}

enum ObjectiveModification {
    DisplayName,
    DisplayAutoUpdate,
    RenderType(RenderType),
}

struct ObjectiveModifyExecutor(ObjectiveModification);

impl CommandExecutor for ObjectiveModifyExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?;
        let world = context.world();
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut objective = ObjectiveArgumentType::objective_or_error(&scoreboard, name)?.clone();
        // ScoreboardCommand.setDisplayName/setDisplayAutoUpdate/setRenderType skip unchanged values.
        let (key, arguments) = match self.0 {
            ObjectiveModification::DisplayName => {
                let display_name = ComponentArgumentType::get(context, ARG_DISPLAY_NAME)?;
                let replaced = scoreboard.discard_objective_display_name(name);
                if objective.display_name == display_name && !replaced {
                    return Ok(0);
                }
                objective.display_name = display_name.clone();
                (
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_DISPLAYNAME,
                    vec![TextComponent::text(name.to_owned()), display_name],
                )
            }
            ObjectiveModification::DisplayAutoUpdate => {
                let value = BoolArgumentType::get(context, ARG_VALUE)?;
                if objective.display_auto_update == value {
                    return Ok(0);
                }
                objective.display_auto_update = value;
                let key = if value {
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_DISPLAYAUTOUPDATE_ENABLE
                } else {
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_DISPLAYAUTOUPDATE_DISABLE
                };
                (
                    key,
                    vec![
                        TextComponent::text(name.to_owned()),
                        objective.display_name.clone(),
                    ],
                )
            }
            ObjectiveModification::RenderType(value) => {
                if objective.render_type == value {
                    return Ok(0);
                }
                objective.render_type = value;
                (
                    java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_RENDERTYPE,
                    vec![objective.display_name.clone()],
                )
            }
        };
        scoreboard.update_objective(world, objective);
        drop(scoreboard);
        context
            .source
            .send_feedback(TextComponent::translate_cross(key, key, arguments), true);
        Ok(0)
    }
}

struct FormatExecutor {
    target: FormatTarget,
    kind: FormatKind,
}

impl CommandExecutor for FormatExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let format = self.kind.get(context)?;
        if matches!(self.target, FormatTarget::Score) {
            return set_score_display(context, DisplayValue::Format(format));
        }
        let name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?;
        let world = context.world();
        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ObjectiveArgumentType::objective_or_error(&scoreboard, name)?;
        let key = if format.is_some() {
            java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_OBJECTIVEFORMAT_SET
        } else {
            java::COMMANDS_SCOREBOARD_OBJECTIVES_MODIFY_OBJECTIVEFORMAT_CLEAR
        };
        scoreboard.set_objective_number_format(world, name, format);
        drop(scoreboard);
        context.source.send_feedback(
            TextComponent::translate_cross(key, key, [TextComponent::text(name.to_owned())]),
            true,
        );
        Ok(0)
    }
}

struct ScoreDisplayExecutor(bool);

impl CommandExecutor for ScoreDisplayExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let name = self
            .0
            .then(|| ComponentArgumentType::get(context, ARG_DISPLAY_NAME))
            .transpose()?;
        set_score_display(context, DisplayValue::Name(name))
    }
}

enum DisplayValue {
    Name(Option<TextComponent>),
    Format(Option<NumberFormat>),
}

fn set_score_display(context: &CommandContext, value: DisplayValue) -> CommandExecutorResult {
    let holders = ScoreHolderArgumentType::get_score_holders(context, ARG_TARGETS)?;
    let name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?;
    let world = context.world();
    let mut scoreboard = world
        .scoreboard
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let objective_display = ObjectiveArgumentType::objective_or_error(&scoreboard, name)?
        .display_name
        .clone();
    // ScoreboardCommand.setScoreDisplay/setScoreNumberFormat create missing, locked scores.
    for holder in &holders {
        match &value {
            DisplayValue::Name(display) => {
                let mut score = scoreboard
                    .get_score(&holder.name, name)
                    .cloned()
                    .unwrap_or_else(|| ScoreboardScore {
                        entity_name: holder.name.clone(),
                        objective_name: name.to_owned(),
                        value: VarInt(0),
                        display_name: None,
                        number_format: None,
                        locked: true,
                    });
                score.display_name.clone_from(display);
                scoreboard.update_score_with_display_name(world, score);
            }
            DisplayValue::Format(format) => {
                scoreboard.set_score_number_format(world, &holder.name, name, format.clone());
            }
        }
    }
    drop(scoreboard);
    let (holder, single) = holder_component(&holders);
    let (key, arguments) = match value {
        DisplayValue::Name(Some(display)) => (
            if single {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NAME_SET_SUCCESS_SINGLE
            } else {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NAME_SET_SUCCESS_MULTIPLE
            },
            vec![display, holder, objective_display],
        ),
        DisplayValue::Name(None) => (
            if single {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NAME_CLEAR_SUCCESS_SINGLE
            } else {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NAME_CLEAR_SUCCESS_MULTIPLE
            },
            vec![holder, objective_display],
        ),
        DisplayValue::Format(Some(_)) => (
            if single {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NUMBERFORMAT_SET_SUCCESS_SINGLE
            } else {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NUMBERFORMAT_SET_SUCCESS_MULTIPLE
            },
            vec![holder, objective_display],
        ),
        DisplayValue::Format(None) => (
            if single {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NUMBERFORMAT_CLEAR_SUCCESS_SINGLE
            } else {
                java::COMMANDS_SCOREBOARD_PLAYERS_DISPLAY_NUMBERFORMAT_CLEAR_SUCCESS_MULTIPLE
            },
            vec![holder, objective_display],
        ),
    };
    context
        .source
        .send_feedback(TextComponent::translate_cross(key, key, arguments), true);
    Ok(holders.len() as i32)
}
