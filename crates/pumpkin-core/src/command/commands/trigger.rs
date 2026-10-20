use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::integer::IntegerArgumentType;
use crate::command::argument_types::objective::ObjectiveArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_data::translation;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Modifies a trigger scoreboard objective.";
const PERMISSION: &str = "minecraft:command.trigger";

const ARG_OBJECTIVE: &str = "objective";
const ARG_VALUE: &str = "value";

const INVALID_TRIGGER_ERROR: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TRIGGER_FAILED_INVALID,
    translation::java::COMMANDS_TRIGGER_FAILED_INVALID,
);

const UNPRIMED_TRIGGER_ERROR: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TRIGGER_FAILED_UNPRIMED,
    translation::java::COMMANDS_TRIGGER_FAILED_UNPRIMED,
);

struct SimpleTriggerExecutor;

impl CommandExecutor for SimpleTriggerExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let player = context.source.player_or_err()?;
        let player_name = player.gameprofile.name.clone();
        let objective_name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?.to_string();

        let world = context.world().clone();

        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(objective) = scoreboard.get_objectives().get(&objective_name) else {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        };

        if objective.criterion != "trigger" {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        }

        let objective_display_name = objective.display_name.clone();

        let Some(mut updated_score) = scoreboard.get_score(&player_name, &objective_name).cloned()
        else {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        };
        if updated_score.locked {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        }

        // Vanilla TriggerCommand mutates the existing ScoreAccess and then locks it.
        let new_value = updated_score.value.0.wrapping_add(1);
        updated_score.value = VarInt(new_value);
        updated_score.locked = true;

        scoreboard.update_score(&world, updated_score);

        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_TRIGGER_SIMPLE_SUCCESS,
                translation::java::COMMANDS_TRIGGER_SIMPLE_SUCCESS,
                [objective_display_name],
            ),
            true,
        );

        Ok(new_value)
    }
}

struct AddTriggerExecutor;

impl CommandExecutor for AddTriggerExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let player = context.source.player_or_err()?;
        let player_name = player.gameprofile.name.clone();
        let objective_name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?.to_string();
        let value = IntegerArgumentType::get(context, ARG_VALUE)?;

        let world = context.world().clone();

        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(objective) = scoreboard.get_objectives().get(&objective_name) else {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        };

        if objective.criterion != "trigger" {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        }

        let objective_display_name = objective.display_name.clone();

        let Some(mut updated_score) = scoreboard.get_score(&player_name, &objective_name).cloned()
        else {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        };
        if updated_score.locked {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        }

        // Vanilla TriggerCommand adds to the existing ScoreAccess, preserving its metadata.
        let new_value = updated_score.value.0.wrapping_add(value);
        updated_score.value = VarInt(new_value);
        updated_score.locked = true;

        scoreboard.update_score(&world, updated_score);

        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_TRIGGER_ADD_SUCCESS,
                translation::java::COMMANDS_TRIGGER_ADD_SUCCESS,
                [
                    objective_display_name,
                    TextComponent::text(value.to_string()),
                ],
            ),
            true,
        );

        Ok(new_value)
    }
}

struct SetTriggerExecutor;

impl CommandExecutor for SetTriggerExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let _guard_effects = crate::world::guard_effects::GuardEffects::new();
        let player = context.source.player_or_err()?;
        let player_name = player.gameprofile.name.clone();
        let objective_name = ObjectiveArgumentType::get(context, ARG_OBJECTIVE)?.to_string();
        let value = IntegerArgumentType::get(context, ARG_VALUE)?;

        let world = context.world().clone();

        let mut scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(objective) = scoreboard.get_objectives().get(&objective_name) else {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        };

        if objective.criterion != "trigger" {
            return Err(INVALID_TRIGGER_ERROR.create_without_context());
        }

        let objective_display_name = objective.display_name.clone();

        let Some(mut updated_score) = scoreboard.get_score(&player_name, &objective_name).cloned()
        else {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        };
        if updated_score.locked {
            return Err(UNPRIMED_TRIGGER_ERROR.create_without_context());
        }

        // Vanilla TriggerCommand sets the existing ScoreAccess value, then locks it.
        updated_score.value = VarInt(value);
        updated_score.locked = true;

        scoreboard.update_score(&world, updated_score);

        context.source.send_feedback(
            TextComponent::translate_cross(
                translation::java::COMMANDS_TRIGGER_SET_SUCCESS,
                translation::java::COMMANDS_TRIGGER_SET_SUCCESS,
                [
                    objective_display_name,
                    TextComponent::text(value.to_string()),
                ],
            ),
            true,
        );

        Ok(value)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Allow,
    ));

    dispatcher.register(
        command("trigger", DESCRIPTION).requires(PERMISSION).then(
            argument(ARG_OBJECTIVE, ObjectiveArgumentType)
                .executes(SimpleTriggerExecutor)
                .then(literal("add").then(
                    argument(ARG_VALUE, IntegerArgumentType::any()).executes(AddTriggerExecutor),
                ))
                .then(literal("set").then(
                    argument(ARG_VALUE, IntegerArgumentType::any()).executes(SetTriggerExecutor),
                )),
        ),
    );
}
