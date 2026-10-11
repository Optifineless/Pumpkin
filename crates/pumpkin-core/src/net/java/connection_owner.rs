use crate::{entity::player::Player, net::ClientPlatform, server::Server};
use std::sync::Arc;
use tracing::error;

/// Finishes Java connection-owner cleanup after packet processing ends.
pub async fn finish_disconnect(player: &Arc<Player>, server: &Arc<Server>) {
    // ServerGamePacketListenerImpl.removePlayerFromWorld closes chat before PlayerList.remove.
    if let ClientPlatform::Java(client) = player.client.as_ref() {
        client.close();
        client.await_tasks().await;
    }
    if let Err(e) = server
        .player_data_storage
        .handle_player_leave(player, server)
        .await
    {
        error!("Failed to save player data on disconnect: {e}");
    }
    player
        .storage_session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Err(e) = server.advancement_manager.save_player(player).await {
        error!("Failed to save player advancement on disconnect: {e}");
    }
    player.client.release_admission();
}
