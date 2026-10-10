use super::{MinecartEntity, MinecartKind};
use crate::{entity::Entity, server::Server};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

// OldMinecartBehavior's named speed constants.
const MAX_SPEED_IN_WATER: f64 = 0.2;
const MAX_SPEED_ON_LAND: f64 = 0.4;

impl MinecartEntity {
    // OldMinecartBehavior.moveAlongTrack/NewMinecartBehavior.calculateBoostTrackSpeed.
    pub(super) const POWERED_RAIL_ACCELERATION: f64 = 0.06;

    // AbstractMinecart.useExperimentalMovement selects both rail and off-rail speed limits.
    pub(super) fn uses_new_behavior(server: &Server) -> bool {
        server
            .datapack_manager
            .is_feature_enabled(server, "minecraft:minecart_improvements")
    }

    // OldMinecartBehavior/NewMinecartBehavior.getMaxSpeed and MinecartFurnace.getMaxSpeed.
    pub(super) fn max_speed(&self, server: &Server) -> f64 {
        let in_water = self.vehicle.entity.touching_water.load(Relaxed);
        let base = if Self::uses_new_behavior(server) {
            let rule = self
                .vehicle
                .entity
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .max_minecart_speed;
            rule as f64 * if in_water { 0.5 } else { 1.0 } / 20.0
        } else if in_water {
            MAX_SPEED_IN_WATER
        } else {
            MAX_SPEED_ON_LAND
        };
        if matches!(self.kind, MinecartKind::Furnace(_)) {
            base * if in_water { 0.75 } else { 0.5 }
        } else {
            base
        }
    }

    pub(super) fn limit_movement(
        &self,
        server: &Server,
        velocity: &mut Vector3<f64>,
        on_rails: bool,
        max_speed: f64,
        powered_rail: Option<Vector3<f64>>,
    ) -> Vector3<f64> {
        if [velocity.x, velocity.y, velocity.z]
            .into_iter()
            .any(|value| !value.is_finite())
        {
            *velocity = Vector3::default();
        }
        if !on_rails {
            // AbstractMinecart.comeOffTrack: clamp horizontal delta, then halve on the ground.
            velocity.x = velocity.x.clamp(-max_speed, max_speed);
            velocity.z = velocity.z.clamp(-max_speed, max_speed);
            if self.vehicle.entity.on_ground.load(Relaxed) {
                *velocity = *velocity * 0.5;
            }
            return *velocity;
        }
        if Self::uses_new_behavior(server) {
            // NewMinecartBehavior.calculateTrackSpeed: slowdown, clamp, then powered boost.
            *velocity = self.rail_slowdown(*velocity, true);
            let speed = velocity.length();
            if speed > max_speed {
                *velocity = velocity.normalize() * max_speed;
            }
            if let Some(direction) = powered_rail {
                if velocity.length() > 0.01 {
                    *velocity = velocity.normalize()
                        * (velocity.length() + Self::POWERED_RAIL_ACCELERATION);
                } else {
                    // Preserve the existing starting impulse along the aligned rail.
                    *velocity = direction * 0.1;
                }
            }
            *velocity
        } else {
            // OldMinecartBehavior.moveAlongTrack caps the step, preserving deltaMovement.
            let scale = if self.vehicle.entity.has_passengers() {
                0.75
            } else {
                1.0
            };
            Vector3::new(
                (scale * velocity.x).clamp(-max_speed, max_speed),
                0.0,
                (scale * velocity.z).clamp(-max_speed, max_speed),
            )
        }
    }

    pub(super) fn movement_slowdown(
        &self,
        velocity: Vector3<f64>,
        on_rails: bool,
        new_behavior: bool,
    ) -> Vector3<f64> {
        if on_rails {
            // NewMinecartBehavior.calculateTrackSpeed already applied natural slowdown.
            if new_behavior {
                velocity
            } else {
                self.rail_slowdown(velocity, false)
            }
        } else if self.vehicle.entity.on_ground.load(Relaxed) {
            // AbstractMinecart.comeOffTrack halves before moving and skips air drag on landing.
            velocity
        } else {
            velocity * 0.95
        }
    }

    fn rail_slowdown(&self, velocity: Vector3<f64>, new_behavior: bool) -> Vector3<f64> {
        if let MinecartKind::Furnace(minecart) = &self.kind {
            minecart.velocity(&self.vehicle.entity, velocity, new_behavior)
        } else if let Some(inventory) = self.container() {
            super::container::velocity(&self.vehicle.entity, inventory, velocity)
        } else if new_behavior {
            experimental_slowdown(&self.vehicle.entity, velocity)
        } else {
            let slowdown = if self.vehicle.entity.has_passengers() {
                0.99
            } else {
                0.96
            };
            velocity * slowdown
        }
    }
}

// AbstractMinecart.applyNaturalSlowdown/NewMinecartBehavior.getSlowdownFactor.
pub(super) fn experimental_slowdown(entity: &Entity, velocity: Vector3<f64>) -> Vector3<f64> {
    let slowdown = if entity.has_passengers() {
        0.997
    } else {
        0.975
    };
    let water = if entity.touching_water.load(Relaxed) {
        f64::from(0.95f32)
    } else {
        1.0
    };
    velocity.multiply(slowdown * water, 0.0, slowdown * water)
}
