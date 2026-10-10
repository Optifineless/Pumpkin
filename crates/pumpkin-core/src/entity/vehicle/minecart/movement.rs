use super::{MinecartEntity, MinecartKind};
use crate::{
    command::commands::gamerule::{MAX_MINECART_SPEED, MIN_MINECART_SPEED},
    entity::{Entity, EntityBase},
    server::Server,
};
use pumpkin_data::{block_properties::RailShape, game_rules::GameRuleRegistry};
use pumpkin_protocol::java::server::play::SPlayerInput;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

// OldMinecartBehavior's named speed constants.
const MAX_SPEED_IN_WATER: f64 = 0.2;
const MAX_SPEED_ON_LAND: f64 = 0.4;
// Mth.equal uses the float EPSILON even when comparing doubles.
const EPSILON: f32 = 1.0e-5;

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
    pub(super) fn max_speed(&self, server: &Server, on_rails: bool) -> f64 {
        let in_water = self.vehicle.entity.touching_water.load(Relaxed);
        let base = if Self::uses_new_behavior(server) {
            let mut rule = self
                .vehicle
                .entity
                .world
                .load()
                .level_info
                .load()
                .game_rules
                .max_minecart_speed
                .clamp(i64::from(MIN_MINECART_SPEED), i64::from(MAX_MINECART_SPEED));
            // Until NewMinecartBehavior.stepAlongTrack is ported, do not skip whole rails.
            if on_rails {
                rule = rule.min(GameRuleRegistry::default().max_minecart_speed);
            }
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
                } else if direction.length_squared() > 0.0 {
                    *velocity = direction * (velocity.length() + 0.2);
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

    // OldMinecartBehavior.moveAlongTrack/NewMinecartBehavior.calculatePlayerInputSpeed.
    pub(super) fn apply_player_input(&self, on_rails: bool) {
        let entity = &self.vehicle.entity;
        let velocity = entity.velocity.load();
        if !on_rails || velocity.x.mul_add(velocity.x, velocity.z * velocity.z) >= 0.01 {
            return;
        }
        let intent = {
            let Ok(passengers) = entity.passengers.try_lock() else {
                return;
            };
            let Some(player) = passengers
                .first()
                .and_then(|passenger| passenger.get_player())
            else {
                return;
            };
            // ServerPlayer.getLastClientMoveIntent cancels opposing keys and normalizes diagonals.
            let input = player.last_input.load(Relaxed);
            let left = i32::from(input & SPlayerInput::LEFT != 0)
                - i32::from(input & SPlayerInput::RIGHT != 0);
            let forward = i32::from(input & SPlayerInput::FORWARD != 0)
                - i32::from(input & SPlayerInput::BACKWARD != 0);
            player.get_entity().movement_input_to_velocity(
                Vector3::new(f64::from(left), 0.0, f64::from(forward)),
                1.0,
            )
        };
        if intent.length_squared() > 0.0 {
            entity.velocity.store(velocity + intent * 0.001);
            entity.send_velocity();
        }
    }

    // AbstractMinecart.getRedstoneDirection: only straight powered rails have end-block starts.
    pub(super) fn redstone_direction(&self, pos: BlockPos, shape: RailShape) -> Vector3<f64> {
        let direction = match shape {
            RailShape::EastWest => Vector3::new(1, 0, 0),
            RailShape::NorthSouth => Vector3::new(0, 0, 1),
            _ => return Vector3::default(),
        };
        let world = self.vehicle.entity.world.load();
        if world
            .get_block_state(&BlockPos(pos.0 - direction))
            .is_solid_block()
        {
            direction.to_f64()
        } else if world
            .get_block_state(&BlockPos(pos.0 + direction))
            .is_solid_block()
        {
            direction.to_f64() * -1.0
        } else {
            Vector3::default()
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

impl Entity {
    /// Preserves horizontal minecart momentum while applying collision and block speed factors.
    pub(crate) fn apply_minecart_movement_velocity(
        &self,
        motion: Vector3<f64>,
        movement: Vector3<f64>,
        block_speed_factor: f64,
    ) {
        // Entity.move/restituteMovementAfterCollisions preserves unobstructed horizontal delta.
        let mut velocity = self.velocity.load();
        if (motion.x - movement.x).abs() >= f64::from(EPSILON) {
            velocity.x = 0.0;
        }
        // Keep Pumpkin's vertical landing/bounce path; rail clamping only changes horizontal steps.
        velocity.y = movement.y;
        if (motion.z - movement.z).abs() >= f64::from(EPSILON) {
            velocity.z = 0.0;
        }
        self.velocity.store(velocity * block_speed_factor);
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
