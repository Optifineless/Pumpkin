use std::sync::{Arc, Weak};

use crate::entity::player::Player;

use super::Server;

impl Server {
    // Called under the shared scoreboard owner, after capturing this session's snapshot.
    pub(crate) fn register_scoreboard_player(&self, player: &Arc<Player>) {
        let weak = Arc::downgrade(player);
        self.scoreboard_audience.rcu(|current| {
            let mut next: Vec<_> = current
                .iter()
                .filter(|entry| entry.strong_count() != 0 && !entry.ptr_eq(&weak))
                .cloned()
                .collect();
            next.push(weak.clone());
            next
        });
    }

    pub(crate) fn unregister_scoreboard_player(&self, player: &Player) {
        self.scoreboard_audience.rcu(|current| {
            current
                .iter()
                .filter(|entry| entry.strong_count() != 0 && !std::ptr::eq(entry.as_ptr(), player))
                .cloned()
                .collect::<Vec<_>>()
        });
    }

    pub(crate) fn scoreboard_players(&self) -> Vec<Arc<Player>> {
        // PlayerList retains recipients throughout ServerPlayer.changeDimension/respawn.
        self.scoreboard_audience
            .load()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|player| !player.client.closed())
            .collect()
    }
}
