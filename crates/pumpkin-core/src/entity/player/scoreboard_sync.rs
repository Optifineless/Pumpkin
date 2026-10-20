use super::Player;

impl Player {
    pub(super) fn send_server_scoreboard(&self) {
        let world = self.world();
        let scoreboard = world
            .scoreboard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        scoreboard.send_to_player(self);
        // PlayerList.updateEntireScoreboard completes before later mutations reach
        // this session. Keep registration and its snapshot under the same owner.
        if let Some(server) = world.server.upgrade()
            && let Some(player) = world.get_player_by_uuid(self.gameprofile.id)
            && std::ptr::eq(self, player.as_ref())
        {
            server.register_scoreboard_player(&player);
        }
    }
}
