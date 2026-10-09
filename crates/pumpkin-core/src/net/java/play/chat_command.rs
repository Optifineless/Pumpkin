#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub async fn handle_chat_command(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        command: &SChatCommand<'_>,
    ) {
        if !self.try_handle_chat(player, command.command, true).await {
            return;
        }
        let command_str = command.command.strip_prefix('/').unwrap_or(command.command);
        // ServerGamePacketListenerImpl.performUnsignedChatCommand uses parsed requirements.
        if server.basic_config.allow_chat_reports
            && Self::command_requires_signature(player, server, command_str)
        {
            player.send_system_message(&TextComponent::translate(
                "chat.disabled.invalid_command_signature",
                [],
            ));
            self.check_session_spam(player, server, crate::entity::player::SpamType::Command);
            return;
        }
        self.execute_chat_command(player, server, command_str, Arc::default())
            .await;
        self.check_session_spam(player, server, crate::entity::player::SpamType::Command);
    }

    pub(in crate::net::java) async fn execute_chat_command(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        input: &str,
        signing_context: Arc<
            std::collections::HashMap<String, crate::net::chat::PlayerChatMessage>,
        >,
    ) {
        // PlayerList.respawn replaces the player captured by performUnsignedChatCommand.
        let life = {
            let _owner = player.living_entity.own_damage();
            let life = crate::entity::living::PlayerTickLife::capture(player.as_ref());
            if !life.is_current() {
                return;
            }
            life
        };
        send_cancellable! {{
            server;
            PlayerCommandSendEvent {
                player: player.clone(),
                command: input.to_owned(),
                cancelled: false
            };
            'after: {
                {
                    let _owner = player.living_entity.own_damage();
                    if !life.is_current() { return; }
                }
                // Commands.performCommand may block on plugins that acquire sender ownership.
                let command = event.command;
                if self.is_closed() { return; }
                if server.basic_config.allow_chat_reports && Self::command_requires_signature(player, server, &command)
                    && (signing_context.is_empty() || command != input)
                {
                    player.send_system_message(&TextComponent::translate("chat.disabled.invalid_command_signature", []));
                    return;
                }
                let mut source = player.get_command_source(server);
                source.signing_context = signing_context;
                let dispatcher = server.command_dispatcher.load_full();
                #[cfg(test)]
                crate::entity::living::damage_transaction::test_hooks::reach(
                    crate::entity::living::damage_transaction::test_hooks::Point::CommandDispatch,
                );
                {
                    let _owner = player.living_entity.own_damage();
                    if !life.is_current() { return; }
                }
                dispatcher.handle_command(&source, &command);
                if server.advanced_config.commands.log_console {
                    info!("Player ({}): executed command /{}", player.gameprofile.name, command);
                }
            }
        }}
    }
}
