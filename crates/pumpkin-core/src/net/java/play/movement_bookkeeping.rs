use crate::{
    entity::{EntityBase, living::FallContext, player::Player},
    net::java::JavaClient,
};
use pumpkin_util::{GameMode, math::vector3::Vector3};
use std::sync::atomic::Ordering;

impl JavaClient {
    pub(super) fn begin_upward_fall_reset(
        player: &Player,
        movement: Vector3<f64>,
        on_ground: bool,
    ) -> Option<(FallContext, f32)> {
        // Grounded movement already resets through fall's landing claim and descent checks.
        if movement.y <= 0.0 || on_ground {
            return None;
        }
        let living = &player.living_entity;
        let _owner = living.own_damage();
        Some((FallContext::capture(living), living.fall_distance.load()))
    }

    pub(super) fn finish_upward_fall_reset(player: &Player, reset: Option<(FallContext, f32)>) {
        let Some((context, distance)) = reset else {
            return;
        };
        let living = &player.living_entity;
        let _owner = living.own_damage();
        if context.is_current(living, player) && living.fall_distance.load() == distance {
            // ServerGamePacketListenerImpl.handlePlayerPositionChange resets after fall accounting.
            living.fall_distance.store(0.0);
        }
    }

    // ServerGamePacketListenerImpl.handlePlayerPositionChange uses the same takeoff for every variant.
    pub(super) fn check_jump(player: &Player, delta_y: f64, on_ground: bool) {
        // Claim the takeoff before jump dispatches reentrant exhaustion callbacks.
        let was_on_ground = player
            .get_entity()
            .on_ground
            .swap(on_ground, Ordering::Relaxed);
        if was_on_ground && !on_ground && delta_y > 0.0 {
            player.jump();
        }
    }

    pub(super) fn check_player_fall(player: &Player, movement: Vector3<f64>, on_ground: bool) {
        player
            .get_entity()
            .check_supporting_block(player, on_ground, movement);
        if on_ground
            && movement == Vector3::new(0.0, 0.0, 0.0)
            && player.get_entity().supporting_block_pos.load().is_none()
        {
            // A ground claim without movement or support must not consume a midair fall.
            return;
        }
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
                movement.y,
                on_ground,
                player.gamemode.load() == GameMode::Creative,
            );
        }
    }
}
