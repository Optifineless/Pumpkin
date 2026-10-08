use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::integer::IntegerArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::{ResultValueTaker, ReturnValue, ReturnValueCallable};
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult, RedirectModifier, Redirection};
use crate::data::datapack::{current_function_frame, discard_function_tail, return_from_function};
use std::sync::Arc;

const DESCRIPTION: &str = "Controls execution flow in functions and sets return values.";
const PERMISSION: &str = "minecraft:command.return";

struct ReturnValueExecutor;

impl CommandExecutor for ReturnValueExecutor {
    fn is_custom(&self) -> bool {
        true
    }

    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let value = IntegerArgumentType::get(context, "value")?;
        // ReturnCommand.ReturnValueCustomExecutor returns and discards the current frame.
        context
            .source
            .command_result_taker
            .call(ReturnValue::Success(value));
        if let Some(id) = current_function_frame() {
            return_from_function(id, ReturnValue::Success(value));
        }
        Ok(value)
    }
}

struct ReturnFailExecutor;

impl CommandExecutor for ReturnFailExecutor {
    fn is_custom(&self) -> bool {
        true
    }

    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        context
            .source
            .command_result_taker
            .call(ReturnValue::Failure);
        if let Some(id) = current_function_frame() {
            return_from_function(id, ReturnValue::Failure);
        }
        Ok(0)
    }
}

struct ReturnToFrame(usize);

impl ReturnValueCallable for ReturnToFrame {
    fn returned_function_frame(&self) -> Option<usize> {
        Some(self.0)
    }

    fn call(&self, value: ReturnValue) {
        return_from_function(self.0, value);
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("return", DESCRIPTION)
            .requires(PERMISSION)
            .then(argument("value", IntegerArgumentType::any()).executes(ReturnValueExecutor))
            .then(literal("fail").executes(ReturnFailExecutor))
            .then(literal("run").redirect_with_modifier(
                Redirection::Root,
                RedirectModifier::CustomUncharged(Arc::new(|context| {
                    // ReturnCommand.ReturnFromCommandCustomModifier discards before continuation.
                    let mut source = (*context.source).clone();
                    if let Some(id) = current_function_frame() {
                        discard_function_tail(id);
                        source =
                            source.merge_command_result_taker(&ResultValueTaker(vec![Arc::new(
                                ReturnToFrame(id),
                            )]));
                    }
                    Ok(vec![Arc::new(source)])
                })),
            )),
    );
}
