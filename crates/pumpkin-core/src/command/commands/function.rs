use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::CommandSource;
use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use crate::data::datapack::FunctionRunError;
use std::sync::Arc;

const DESCRIPTION: &str = "Runs commands found in the corresponding function files.";
const PERMISSION: &str = "minecraft:command.function";

static ERROR_UNKNOWN_FUNCTION: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
);

static ERROR_NO_FUNCTIONS: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
    translation::java::COMMANDS_FUNCTION_SCHEDULED_NO_FUNCTIONS,
);

struct FunctionSuggestionProvider;

impl SuggestionProvider for FunctionSuggestionProvider {
    fn suggest(
        &self,
        context: &CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult {
        let server = context.server();
        let function_names = server.datapack_manager.get_function_names();
        for name in function_names {
            builder = builder.suggest(name);
        }
        builder.build()
    }
}

struct FunctionExecutor;

impl CommandExecutor for FunctionExecutor {
    fn execute_batch(
        &self,
        context: &CommandContext,
        sources: &[Arc<CommandSource>],
    ) -> Option<CommandExecutorResult> {
        Some(Self::queue_sources(context, sources))
    }

    fn is_custom(&self) -> bool {
        true
    }

    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        Self::queue_sources(context, std::slice::from_ref(&context.source))
    }
}

impl FunctionExecutor {
    fn queue_sources(
        context: &CommandContext,
        sources: &[Arc<CommandSource>],
    ) -> CommandExecutorResult {
        let name_str = FunctionArgumentType::get(context, "name")?;
        let server = context.server();

        let executed_count = match server
            .datapack_manager
            .execute_function_sources(server, sources, name_str)
        {
            Ok(executed_count) => executed_count,
            Err(FunctionRunError::ChainLimitExceeded) => {
                // ExecutionContext.runCommandQueue only logs queue overflow.
                return Ok(0);
            }
            Err(FunctionRunError::Unknown(_)) => {
                return Err(ERROR_UNKNOWN_FUNCTION
                    .create_without_context(TextComponent::text(name_str.to_string())));
            }
            Err(FunctionRunError::EmptyTag(_)) => {
                return Err(ERROR_NO_FUNCTIONS
                    .create_without_context(TextComponent::text(name_str.to_string())));
            }
        };

        Ok(executed_count as i32)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("function", DESCRIPTION).requires(PERMISSION).then(
            argument("name", FunctionArgumentType)
                .suggests(FunctionSuggestionProvider)
                .executes(FunctionExecutor),
        ),
    );
}
