use crate::{
    entity::{EntityBase, player::Player},
    net::java::JavaClient,
};
use pumpkin_util::GameMode;
use std::sync::atomic::Ordering;

impl JavaClient {
    // ServerGamePacketListenerImpl.handlePlayerPositionChange uses the same takeoff for every variant.
    pub(super) fn check_jump(player: &Player, delta_y: f64, on_ground: bool) {
        if player.get_entity().on_ground.load(Ordering::Relaxed) && !on_ground && delta_y > 0.0 {
            player.jump();
        }
    }

    pub(super) fn check_player_fall(player: &Player, delta_y: f64, on_ground: bool) {
        let flying = player
            .abilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .flying;
        if !flying
            && player.living_entity.health.load() > 0.0
            && !player.living_entity.dead.load(Ordering::Relaxed)
        {
            player.living_entity.fall(
                player,
                delta_y,
                on_ground,
                player.gamemode.load() == GameMode::Creative,
            );
        }
    }
}
