use crate::{
    entity::player::{ChatMode, Player},
    net::java::JavaClient,
};
use pumpkin_util::text::TextComponent;

impl JavaClient {
    // ServerGamePacketListenerImpl.tryHandleChat, shared by chat and both command forms.
    pub(in crate::net::java) async fn try_handle_chat(
        &self,
        player: &Player,
        message: &str,
        command: bool,
    ) -> bool {
        if self.is_closed() {
            return false;
        }
        if message.chars().any(|c| c == '§' || c < ' ' || c == '\x7f') {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.illegal_characters",
                [],
            ))
            .await;
            return false;
        }
        if !command && matches!(player.config.load().chat_mode, ChatMode::Hidden) {
            player.send_system_message(&TextComponent::translate("chat.disabled.options", []));
            return false;
        }
        player.update_last_action_time();
        true
    }
}
