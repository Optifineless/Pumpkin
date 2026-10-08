use super::JavaClient;
use crate::{entity::player::Player, net::ClientPlatform};
use pumpkin_protocol::bedrock::client::CPlayerSkin;
use std::sync::Arc;

impl JavaClient {
    pub(super) fn start_skin_download(&self, player: &Arc<Player>) {
        let Some(server) = player.world().server.upgrade() else {
            return;
        };
        if !server.advanced_config.networking.bedrock.enabled {
            return;
        }
        // PlayerList.placeNewPlayer forwards textures, never waits for an image download.
        // Fork skin conversion keeps a weak player and updates Bedrock viewers after joining.
        let properties = player.gameprofile.properties.load_full();
        let player = Arc::downgrade(player);
        let close = self.close_token.clone();
        self.spawn_task(async move {
            let skin = tokio::select! {
                () = close.cancelled() => return,
                result = crate::entity::player_skin::fetch_skin(&properties) => result,
            };
            let Some(skin) = skin else {
                return;
            };
            let Some(player) = player.upgrade() else {
                return;
            };
            if close.is_cancelled() || player.client.closed() {
                return;
            }
            player.bedrock_skin.store(Arc::new(skin));
            let skin = player.bedrock_skin.load_full();
            let packet = CPlayerSkin {
                uuid: player.gameprofile.id,
                skin: &skin,
                new_skin_name: &skin.skin_id,
                old_skin_name: "",
            };
            if close.is_cancelled() || player.client.closed() {
                return;
            }
            for viewer in player.world().players.load().iter() {
                if let ClientPlatform::Bedrock(client) = viewer.client.as_ref() {
                    client.try_enqueue_client_packet(&packet);
                }
            }
        });
    }
}
