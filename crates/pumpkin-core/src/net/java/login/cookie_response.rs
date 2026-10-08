#[allow(clippy::wildcard_imports)]
use super::*;

impl PendingConnection {
    pub async fn handle_login_cookie_response(&mut self, _packet: &SLoginCookieResponse<'_>) {
        // ServerLoginPacketListenerImpl.handleCookieResponse.
        self.kick(TextComponent::translate(
            "multiplayer.disconnect.unexpected_query_response",
            [],
        ))
        .await;
    }
}
