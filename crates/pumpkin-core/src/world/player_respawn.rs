use super::World;
use crate::entity::player::Player;
use std::sync::Arc;

impl World {
    /// Tests membership by instance, so an obsolete session cannot act on a rejoin.
    pub(crate) fn contains_player_instance(&self, player: &Arc<Player>) -> bool {
        self.players.load().iter().any(|p| Arc::ptr_eq(p, player))
    }

    /// Respawn may proceed only while its connection and world instance are current.
    pub(crate) fn can_respawn_player(&self, player: &Arc<Player>) -> bool {
        !player.client.closed()
            && !player.living_entity.entity.is_removed()
            && std::ptr::eq(player.world().as_ref(), self)
            && self.contains_player_instance(player)
    }

    pub(super) async fn transfer_respawning_player(
        &self,
        player: &Arc<Player>,
        destination: &Arc<Self>,
    ) -> bool {
        // PlayerList.respawn is serial with PlayerList.remove in vanilla.
        // The client owns this task and joins it before saving/removing on logout.
        if !self.can_respawn_player(player) || self.remove_player(player, false).await.is_none() {
            return false;
        }
        player.unload_watched_chunks(self).await;
        player.change_world_chunks(&self.level, destination);
        player.living_entity.entity.set_world(destination.clone());

        let mut published = false;
        destination.players.rcu(|current_list| {
            published = false;
            let mut new_list = (**current_list).clone();
            if !player.client.closed()
                && !player.living_entity.entity.is_removed()
                && !new_list
                    .iter()
                    .any(|p| p.gameprofile.id == player.gameprofile.id)
            {
                new_list.push(player.clone());
                published = true;
            }
            new_list
        });
        published
    }
}
