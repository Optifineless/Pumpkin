use std::sync::Arc;

use super::Player;
use crate::{entity::EntityBase, net::ClientPlatform, server::Server};

impl Player {
    /// Drains Java connection work before capturing the final save and removing membership.
    pub(crate) async fn finish_java_connection(self: &Arc<Self>, server: &Arc<Server>) {
        // ServerGamePacketListenerImpl.onDisconnect -> PlayerList.remove runs after teleport.
        if let ClientPlatform::Java(client) = self.client.as_ref() {
            self.get_entity().invalidate_portal_travel();
            client.close();
            client.await_tasks().await;
        }
        if let Err(error) = server
            .player_data_storage
            .handle_player_leave(self, server)
            .await
        {
            tracing::error!("Failed to save player data on disconnect: {error}");
        }
        self.storage_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Err(error) = server.advancement_manager.save_player(self).await {
            tracing::error!("Failed to save player advancement on disconnect: {error}");
        }
        self.client.release_admission();
    }
}
