use std::sync::Arc;

use crate::entity::player::Player;

const SLEEP_DURATION_REQUIRED: u8 = 100;

pub(super) fn are_enough_sleeping(players: &[Arc<Player>], sleep_percentage_needed: i64) -> bool {
    let mut active_players = 0;
    let mut sleeping_players = 0;

    // Mirrors vanilla 26.3 SleepStatus.update: Spectators are not active players.
    for player in players {
        if player.gamemode.load() == pumpkin_util::GameMode::Spectator {
            continue;
        }

        active_players += 1;
        if player
            .sleeping_since
            .load()
            .is_some_and(|since| since >= SLEEP_DURATION_REQUIRED)
        {
            sleeping_players += 1;
        }
    }

    // Mirrors SleepStatus.sleepersNeeded, including its minimum of one sleeper.
    let sleepers_needed =
        ((active_players as f64 * sleep_percentage_needed as f64) / 100.0).ceil() as usize;
    sleeping_players >= sleepers_needed.max(1)
}
