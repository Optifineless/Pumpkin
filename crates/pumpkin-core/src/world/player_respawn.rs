use super::World;
use crate::entity::{EntityBase, player::Player};
use std::sync::Arc;

impl World {
    /// PlayerList.remove must address the current player instance, not a same-UUID rejoin.
    pub(crate) fn contains_player_instance(&self, player: &Player) -> bool {
        self.players
            .load()
            .iter()
            .any(|current| std::ptr::eq(current.as_ref(), player))
    }

    /// PlayerList.respawn may continue only while its connection and world instance are current.
    pub(crate) fn can_respawn_player(&self, player: &Arc<Player>) -> bool {
        !player.client.closed()
            && !player.get_entity().is_removed()
            && std::ptr::eq(player.world().as_ref(), self)
            && self.contains_player_instance(player)
    }

    pub(super) async fn transfer_respawning_player(
        self: &Arc<Self>,
        player: &Arc<Player>,
        destination: &Arc<Self>,
    ) -> bool {
        if !self.can_respawn_player(player) {
            return false;
        }
        let destination_has_session = destination.players.load().iter().any(|current| {
            current.gameprofile.id == player.gameprofile.id && !Arc::ptr_eq(current, player)
        });
        if destination_has_session || self.remove_player(player, false).await.is_none() {
            return false;
        }

        player.unload_watched_chunks(self).await;
        if !owns_unpublished_session(self, player) {
            return false;
        }

        player.change_world_chunks(&self.level, destination);
        player.get_entity().set_world(destination.clone());

        let mut published = false;
        destination.players.rcu(|current_list| {
            published = false;
            let mut new_list = (**current_list).clone();
            if owns_unpublished_session(destination, player)
                && !new_list
                    .iter()
                    .any(|current| current.gameprofile.id == player.gameprofile.id)
            {
                new_list.push(player.clone());
                published = true;
            }
            new_list
        });
        if published {
            return true;
        }

        // PlayerList.respawn leaves the source world intact when destination admission fails.
        if owns_unpublished_session(destination, player) {
            // A competing destination session must not leave this active instance detached.
            player.change_world_chunks(&destination.level, self);
            player.get_entity().set_world(self.clone());
            self.restore_respawn_source_membership(player);
        }
        false
    }

    fn restore_respawn_source_membership(&self, player: &Arc<Player>) {
        let mut restored = false;
        self.players.rcu(|current_list| {
            restored = false;
            let mut new_list = (**current_list).clone();
            if owns_unpublished_session(self, player)
                && !new_list
                    .iter()
                    .any(|current| current.gameprofile.id == player.gameprofile.id)
            {
                new_list.push(player.clone());
                restored = true;
            }
            new_list
        });
        if restored {
            self.add_arriving_player(player);
        }
    }
}

fn owns_unpublished_session(world: &World, player: &Arc<Player>) -> bool {
    !player.client.closed()
        && !player.get_entity().is_removed()
        && std::ptr::eq(player.world().as_ref(), world)
}
