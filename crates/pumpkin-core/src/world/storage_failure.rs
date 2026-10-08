use super::World;
use crate::entity::player::Player;
use pumpkin_util::{math::vector2::Vector2, text::TextComponent};
use pumpkin_world::level::SyncChunk;

impl World {
    pub(super) async fn load_player_chunk(
        &self,
        pos: Vector2<i32>,
        player: &Player,
    ) -> Option<SyncChunk> {
        // IOWorker.loadAsync failures must complete spawn/respawn instead of hanging
        // or installing an empty replacement for saved terrain.
        match self.level.get_or_fetch_chunk(pos, Clone::clone).await {
            Ok(chunk) => Some(chunk),
            Err(error) => {
                tracing::error!("Cannot load player destination {pos:?}: {error}");
                player.kick(
                    crate::net::DisconnectReason::UnrecoverableError,
                    &TextComponent::text("Destination chunk could not be loaded"),
                );
                None
            }
        }
    }
}
