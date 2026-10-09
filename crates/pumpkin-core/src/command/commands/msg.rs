use pumpkin_data::{chat_type::ChatType, world::MSG_COMMAND_INCOMING};
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};

use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::entity::EntityBase;

const NAMES: [&str; 3] = ["msg", "tell", "w"];
const DESCRIPTION: &str = "Sends a private message to one or more players.";
const PERMISSION: &str = "minecraft:command.msg";

struct MsgExecutor;

impl CommandExecutor for MsgExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let targets = EntityArgumentType::get_players(context, "targets")?;
        let msg = StringArgumentType::get(context, "message")?;

        let sender_name = &context.source.display_name;
        // MsgCommand.sendMessage uses identical source routing for signed and unsigned messages.
        let unsigned = crate::net::chat::PlayerChatMessage::system(msg.to_owned());
        let message = context
            .source
            .signing_context
            .get("message")
            .unwrap_or(&unsigned);
        let tracked = crate::net::chat::OutgoingChatMessage::create(message.clone());
        for target in &targets {
            context.source.send_chat_message(
                &tracked,
                ChatType::MsgCommandOutgoing,
                Some(&target.get_display_name()),
            );
            tracked.send_to_player(
                target,
                false,
                (MSG_COMMAND_INCOMING + 1).into(),
                sender_name,
                None,
            );
        }

        Ok(targets.len() as i32)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Zero),
    ));

    for name in NAMES {
        dispatcher.register(
            command(name, DESCRIPTION).requires(PERMISSION).then(
                argument("targets", EntityArgumentType::Players).then(
                    argument(
                        "message",
                        crate::net::java::signed_commands::MessageArgument,
                    )
                    .executes(MsgExecutor),
                ),
            ),
        );
    }
}
