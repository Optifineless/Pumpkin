use super::JavaClient;
use crate::{
    entity::player::Player,
    net::chat::{PlayerChatMessage, SignedMessageBody},
    server::Server,
};
use pumpkin_command::{
    argument_types::argument_type::{ArgumentType, JavaClientArgumentType},
    context::command_context::CommandContextBuilder,
    errors::command_syntax_error::CommandSyntaxError,
    node::attached::AttachedNode,
    source::CommandSource,
    string_reader::StringReader,
};
use pumpkin_protocol::java::server::play::SChatCommandSigned;
use pumpkin_util::text::TextComponent;
use std::{collections::HashMap, sync::Arc};

/// Marks a greedy message argument as signable in the advertised command tree.
pub struct MessageArgument;

impl<S: CommandSource> ArgumentType<S> for MessageArgument {
    type Item = String;

    fn parse(&self, reader: &mut StringReader) -> Result<String, CommandSyntaxError> {
        let value = reader.remaining_part().to_owned();
        reader.set_cursor(reader.total_length());
        Ok(value)
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::Message
    }
}

fn signable_arguments<S: CommandSource>(
    mut context: &CommandContextBuilder<'_, S>,
    input: &str,
) -> Vec<(String, String)> {
    // SignableCommand.of / ArgumentVisitor.visitArguments include redirected child contexts.
    let mut arguments = Vec::new();
    loop {
        for parsed in &context.nodes {
            if let AttachedNode::Argument(node) = &context.dispatcher.tree[parsed.node]
                && matches!(
                    node.meta.argument_type.client_side_parser(),
                    JavaClientArgumentType::Message
                )
                && let Some(value) = context.arguments.get(node.meta.name.as_ref())
            {
                arguments.push((
                    node.meta.name.to_string(),
                    value.range.substring_slice(input).to_owned(),
                ));
            }
        }
        let Some(child) = context.child.as_deref() else {
            break;
        };
        context = child;
    }
    arguments
}

impl JavaClient {
    pub(super) fn command_requires_signature(
        player: &Arc<Player>,
        server: &Arc<Server>,
        input: &str,
    ) -> bool {
        let dispatcher = server.command_dispatcher.load_full();
        let parsed = dispatcher.parse_input(input, &player.get_command_source(server));
        !signable_arguments(&parsed.context, input).is_empty()
    }

    pub(super) async fn handle_signed_command(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        signed: SChatCommandSigned<'_>,
        seen: Vec<Box<[u8]>>,
    ) {
        if !self.try_handle_chat(player, signed.command, true).await {
            return;
        }
        let dispatcher = server.command_dispatcher.load_full();
        let parsed = dispatcher.parse_input(signed.command, &player.get_command_source(server));
        let arguments = signable_arguments(&parsed.context, signed.command);
        let context = if server.basic_config.allow_chat_reports {
            match self.verify_command_arguments(player, &signed, &arguments, &seen) {
                Ok(context) => context,
                Err(reason) => {
                    player.send_system_message(&TextComponent::translate(reason, []));
                    self.check_session_spam(
                        player,
                        server,
                        crate::entity::player::SpamType::Command,
                    );
                    return;
                }
            }
        } else {
            HashMap::new()
        };
        // performSignedChatCommand installs CommandSigningContext before command execution.
        self.execute_chat_command(player, server, signed.command, Arc::new(context))
            .await;
        self.check_session_spam(player, server, crate::entity::player::SpamType::Command);
    }

    fn verify_command_arguments(
        &self,
        player: &Player,
        signed: &SChatCommandSigned<'_>,
        arguments: &[(String, String)],
        seen: &[Box<[u8]>],
    ) -> Result<HashMap<String, PlayerChatMessage>, &'static str> {
        let session = {
            let session = player
                .chat_session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (*session).clone()
        };
        let mut chain = self
            .chat_order
            .chain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        collect_signed_arguments(
            &mut chain,
            player.gameprofile.id,
            &session,
            signed,
            arguments,
            seen,
        )
    }
}

fn collect_signed_arguments(
    chain: &mut super::play::chat_chain::SignedMessageChain,
    sender: uuid::Uuid,
    session: &crate::entity::player::ChatSession,
    signed: &SChatCommandSigned<'_>,
    arguments: &[(String, String)],
    seen: &[Box<[u8]>],
) -> Result<HashMap<String, PlayerChatMessage>, &'static str> {
    // ServerGamePacketListenerImpl.collectSignedArguments consumes links before execution/refusal.
    let mut supplied = HashMap::new();
    for signature in &signed.argument_signatures {
        let Some((_, value)) = arguments.iter().find(|(name, _)| name == signature.name) else {
            chain.set_chain_broken();
            return Err("chat.disabled.invalid_command_signature");
        };
        let std::collections::hash_map::Entry::Vacant(entry) =
            supplied.entry(signature.name.to_owned())
        else {
            chain.set_chain_broken();
            return Err("chat.disabled.invalid_command_signature");
        };
        let message = chain.unpack(
            sender,
            session,
            SignedMessageBody::new(value.clone(), signed.timestamp, signed.salt, seen.to_vec()),
            Some(signature.signature),
        )?;
        entry.insert(message);
    }
    if arguments
        .iter()
        .any(|(name, _)| !supplied.contains_key(name))
    {
        return Err("chat.disabled.invalid_command_signature");
    }
    Ok(supplied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_command::{
        argument_builder::{ArgumentBuilder, argument, command},
        node::{Redirection, dispatcher::CommandDispatcher},
        source::DummySource,
    };
    use pumpkin_protocol::{VarInt, java::server::play::ArgumentSignature};

    #[test]
    fn refused_command_consumes_verified_links_and_omissions_cannot_execute() {
        let session = crate::entity::player::ChatSession::new(
            uuid::Uuid::nil(),
            i64::MAX,
            include_bytes!("play/fixtures/chat-key.der")
                .as_slice()
                .into(),
            Box::new([]),
        );
        let mut chain = super::super::play::chat_chain::SignedMessageChain::default();
        chain.reset(&session).unwrap();
        let mut packet = SChatCommandSigned {
            command: "say hello",
            timestamp: 1000,
            salt: 0,
            argument_signatures: vec![],
            message_count: VarInt(0),
            acknowledged: &[0; 3],
            checksum: 0,
        };
        let arguments = vec![("message".into(), "hello".into())];
        assert!(
            collect_signed_arguments(
                &mut chain,
                uuid::Uuid::nil(),
                &session,
                &packet,
                &arguments,
                &[]
            )
            .is_err()
        );
        packet.argument_signatures.push(ArgumentSignature {
            name: "message",
            signature: include_bytes!("play/fixtures/chat-signature.bin"),
        });
        // A valid first argument is consumed even when another required argument is omitted.
        let mut incomplete = arguments;
        incomplete.push(("other".into(), "missing".into()));
        assert!(
            collect_signed_arguments(
                &mut chain,
                uuid::Uuid::nil(),
                &session,
                &packet,
                &incomplete,
                &[]
            )
            .is_err()
        );
        // Independent OpenSSL fixture at link index 1, same body, proves the next chat remains valid.
        let next = chain.unpack(
            uuid::Uuid::nil(),
            &session,
            SignedMessageBody::new("hello".into(), 1000, 0, vec![]),
            Some(include_bytes!("play/fixtures/chat-index-one.bin")),
        );
        assert!(next.is_ok());
    }

    #[test]
    fn signature_policy_comes_from_parsed_arguments_including_redirects() {
        let mut dispatcher = CommandDispatcher::<DummySource>::new();
        dispatcher.register(command("say", "").then(argument("message", MessageArgument)));
        dispatcher.register(command("run", "").redirect(Redirection::Root));
        dispatcher.register(command("tellraw", "").then(argument(
            "component",
            pumpkin_command::argument_types::core::string::StringArgumentType::GreedyPhrase,
        )));
        for input in ["say hello", "run say hello"] {
            let parsed = dispatcher.parse_input(input, &DummySource::dummy());
            assert_eq!(
                signable_arguments(&parsed.context, input),
                vec![("message".into(), "hello".into())]
            );
        }
        let parsed = dispatcher.parse_input("tellraw hello", &DummySource::dummy());
        assert!(signable_arguments(&parsed.context, "tellraw hello").is_empty());
    }
}
