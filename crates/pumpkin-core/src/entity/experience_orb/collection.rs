use super::{ExperienceOrbEntity, Player};
use crate::entity::EntityBase;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;
use std::sync::Arc;

/// Collects one random nearby XP orb per eligible player, as in `Player.aiStep`.
/// Run after entity ticks so merging cannot race with collection on another tick worker.
pub fn collect_nearby_orbs(players: &[Arc<Player>], entities: &[Arc<dyn EntityBase>]) {
    for player in players {
        if !player.can_collect_experience() {
            continue;
        }
        let area = pickup_area(player);
        let player_world = player.world();
        let candidates: Vec<_> = entities
            .iter()
            .filter_map(|entity| {
                let orb = entity.cast_any().downcast_ref::<ExperienceOrbEntity>()?;
                // Player.aiStep queries its current level, including after a portal tick.
                (!orb.entity.is_removed()
                    && Arc::ptr_eq(&player_world, &orb.entity.world.load())
                    && orb.entity.bounding_box.load().intersects(&area))
                .then_some(orb)
            })
            .collect();
        if !candidates.is_empty() {
            candidates[rand::rng().random_range(0..candidates.len())].on_player_collision(player);
        }
    }
}

// Player.aiStep uses the player/vehicle union when riding, without vertical inflation.
fn pickup_area(player: &Player) -> BoundingBox {
    let area = player.living_entity.entity.bounding_box.load();
    if let Some(vehicle) = player
        .living_entity
        .entity
        .vehicle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .cloned()
        && !vehicle.get_entity().is_removed()
    {
        let vehicle = vehicle.get_entity().bounding_box.load();
        BoundingBox::new(
            Vector3::new(
                area.min.x.min(vehicle.min.x),
                area.min.y.min(vehicle.min.y),
                area.min.z.min(vehicle.min.z),
            ),
            Vector3::new(
                area.max.x.max(vehicle.max.x),
                area.max.y.max(vehicle.max.y),
                area.max.z.max(vehicle.max.z),
            ),
        )
        .expand(1.0, 0.0, 1.0)
    } else {
        area.expand(1.0, 0.5, 1.0)
    }
}
