use pumpkin_protocol::java::client::play::{
    CInitializeWorldBorder, CSetBorderCenter, CSetBorderLerpSize, CSetBorderSize,
    CSetBorderWarningDelay, CSetBorderWarningDistance,
};

use crate::entity::player::Player;

use super::World;

const MILLIS_PER_TICK: i64 = 50;

#[derive(Clone)]
pub struct Worldborder {
    effects: super::guard_effects::OwnerEffects,
    extent: Option<MovingBorderExtent>,
    pub center_x: f64,
    pub center_z: f64,
    pub old_diameter: f64,
    pub new_diameter: f64,
    /// Remaining transition time in milliseconds for the legacy native API.
    pub speed: i64,
    pub portal_teleport_boundary: i32,
    pub warning_blocks: i32,
    pub warning_time: i32,
    pub damage_per_block: f32,
    pub buffer: f32,
}

#[derive(Clone)]
struct MovingBorderExtent {
    from: f64,
    duration: i64,
    remaining: i64,
    size: f64,
    previous_size: f64,
}

impl MovingBorderExtent {
    fn update(&mut self, target: f64) {
        // WorldBorder.MovingBorderExtent.update/calculateSize advance only on game ticks.
        self.remaining -= 1;
        self.previous_size = self.size;
        let duration = self.duration as f64;
        let progress = (duration - self.remaining as f64) / duration;
        self.size = if progress < 1.0 {
            self.from + progress * (target - self.from)
        } else {
            target
        };
    }
}

impl Worldborder {
    #[must_use]
    pub fn new(
        x: f64,
        z: f64,
        diameter: f64,
        speed: i64,
        warning_blocks: i32,
        warning_time: i32,
    ) -> Self {
        Self {
            effects: super::guard_effects::OwnerEffects::default(),
            extent: None,
            center_x: x,
            center_z: z,
            old_diameter: diameter,
            new_diameter: diameter,
            speed,
            portal_teleport_boundary: 29_999_984,
            warning_blocks,
            warning_time,
            damage_per_block: 0.0,
            buffer: 0.0,
        }
    }

    fn init_client(&self, player: &Player) {
        // ServerLevel's border listener and initial state share the same notification order.
        self.effects.capture(|| {
            player.try_send_client_packet(&CInitializeWorldBorder::new(
                self.center_x,
                self.center_z,
                self.get_size(),
                self.get_lerp_target(),
                self.get_lerp_time().into(),
                self.portal_teleport_boundary.into(),
                self.warning_blocks.into(),
                self.warning_time.into(),
            ));
        });
    }

    // WorldBorder setters mutate state before notifying their listener snapshots.
    pub fn set_center(&mut self, world: &World, x: f64, z: f64) {
        self.center_x = x;
        self.center_z = z;

        self.effects.capture(|| {
            super::guard_effects::broadcast_je(
                world,
                &CSetBorderCenter::new(self.center_x, self.center_z),
            );
        });
    }

    /// Sets the diameter using the legacy native duration in milliseconds, rounded up to ticks.
    pub fn set_diameter(&mut self, world: &World, diameter: f64, speed: Option<i64>) {
        let ticks =
            speed.map(|millis| millis / MILLIS_PER_TICK + i64::from(millis % MILLIS_PER_TICK > 0));
        self.set_diameter_ticks(world, diameter, ticks);
    }

    /// Sets the diameter with a transition duration in game ticks.
    pub fn set_diameter_ticks(&mut self, world: &World, diameter: f64, ticks: Option<i64>) {
        self.old_diameter = self.get_size();
        self.new_diameter = diameter;
        let ticks = ticks.filter(|ticks| *ticks > 0);
        self.extent = ticks
            .filter(|_| self.old_diameter != diameter)
            .map(|duration| MovingBorderExtent {
                from: self.old_diameter,
                duration,
                remaining: duration,
                size: self.old_diameter,
                previous_size: self.old_diameter,
            });
        self.speed = self.get_lerp_time().saturating_mul(MILLIS_PER_TICK);

        match ticks {
            Some(_) => {
                self.effects.capture(|| {
                    super::guard_effects::broadcast_je(
                        world,
                        &CSetBorderLerpSize::new(
                            self.get_size(),
                            self.get_lerp_target(),
                            self.get_lerp_time().into(),
                        ),
                    );
                });
            }
            None => {
                self.effects.capture(|| {
                    super::guard_effects::broadcast_je(
                        world,
                        &CSetBorderSize::new(self.new_diameter),
                    );
                });
            }
        }
    }

    pub fn add_diameter(&mut self, world: &World, offset: f64, speed: Option<i64>) {
        let speed = speed.map(|millis| {
            millis.saturating_add(self.get_lerp_time().saturating_mul(MILLIS_PER_TICK))
        });
        self.set_diameter(world, self.get_size() + offset, speed);
    }

    #[must_use]
    pub fn get_size(&self) -> f64 {
        self.extent
            .as_ref()
            .map_or(self.new_diameter, |extent| extent.size)
    }

    #[must_use]
    pub fn get_lerp_time(&self) -> i64 {
        self.extent.as_ref().map_or(0, |extent| extent.remaining)
    }

    #[must_use]
    pub const fn get_lerp_target(&self) -> f64 {
        self.new_diameter
    }

    pub fn tick(&mut self) {
        if let Some(extent) = &mut self.extent {
            extent.update(self.new_diameter);
            if extent.remaining <= 0 {
                self.extent = None;
                self.old_diameter = self.new_diameter;
            }
            self.speed = self.get_lerp_time().saturating_mul(MILLIS_PER_TICK);
        }
    }

    /// Returns min X/Z and max X/Z, matching `WorldBorder`'s partial-tick bounds.
    #[must_use]
    pub fn bounds(&self, partial_tick: f32) -> (f64, f64, f64, f64) {
        let size = self.extent.as_ref().map_or(self.new_diameter, |extent| {
            extent.previous_size + f64::from(partial_tick) * (extent.size - extent.previous_size)
        });
        let half = size / 2.0;
        let limit = f64::from(self.portal_teleport_boundary);
        (
            (self.center_x - half).clamp(-limit, limit),
            (self.center_z - half).clamp(-limit, limit),
            (self.center_x + half).clamp(-limit, limit),
            (self.center_z + half).clamp(-limit, limit),
        )
    }

    pub fn set_warning_delay(&mut self, world: &World, delay: i32) {
        self.warning_time = delay;

        self.effects.capture(|| {
            super::guard_effects::broadcast_je(
                world,
                &CSetBorderWarningDelay::new(self.warning_time.into()),
            );
        });
    }

    pub fn set_warning_distance(&mut self, world: &World, distance: i32) {
        self.warning_blocks = distance;

        self.effects.capture(|| {
            super::guard_effects::broadcast_je(
                world,
                &CSetBorderWarningDistance::new(self.warning_blocks.into()),
            );
        });
    }

    pub const fn set_damage_buffer(&mut self, buffer: f32) {
        self.buffer = buffer;
    }

    pub const fn set_damage_per_block(&mut self, damage: f32) {
        self.damage_per_block = damage;
    }

    pub fn reset(&mut self, world: &World) {
        self.center_x = 0.0;
        self.center_z = 0.0;
        self.old_diameter = 29_999_984.0;
        self.new_diameter = 29_999_984.0;
        self.speed = 0;
        self.extent = None;
        self.portal_teleport_boundary = 29_999_984;
        self.warning_blocks = 5;
        self.warning_time = 15;
        self.damage_per_block = 0.2;
        self.buffer = 5.0;

        self.effects.capture(|| {
            super::guard_effects::broadcast_je(
                world,
                &CInitializeWorldBorder::new(
                    self.center_x,
                    self.center_z,
                    self.old_diameter,
                    self.new_diameter,
                    self.speed.into(),
                    self.portal_teleport_boundary.into(),
                    self.warning_blocks.into(),
                    self.warning_time.into(),
                ),
            );
        });
    }

    #[must_use]
    pub fn contains(&self, x: f64, z: f64) -> bool {
        let (min_x, min_z, max_x, max_z) = self.bounds(0.0);
        x >= min_x && x < max_x && z >= min_z && z < max_z
    }

    #[must_use]
    pub fn contains_block(&self, x: i32, z: i32) -> bool {
        self.contains(f64::from(x), f64::from(z))
            && self.contains(f64::from(x + 1), f64::from(z + 1))
    }

    #[must_use]
    /// Clamps a random-teleport destination to the vanilla border bounds.
    pub fn clamp_teleport_position(
        &self,
        mut position: pumpkin_util::math::vector3::Vector3<f64>,
    ) -> pumpkin_util::math::vector3::Vector3<f64> {
        // WorldBorder.clampVec3ToBound uses the float constant 1.0E-5F at the upper edges.
        let (min_x, min_z, max_x, max_z) = self.bounds(0.0);
        let clamp = |value: f64, min: f64, max: f64| {
            let max = max - f64::from(1.0e-5f32);
            if value < min { min } else { value.min(max) }
        };
        position.x = clamp(position.x, min_x, max_x);
        position.z = clamp(position.z, min_z, max_z);
        position
    }

    #[must_use]
    pub fn clamp_block(&self, x: i32, z: i32) -> (i32, i32) {
        let (min_x, min_z, max_x, max_z) = self.bounds(0.0);
        // A border narrower than one block spans no block boundary, leaving `max`
        // below `min`. `Ord::clamp` panics on an inverted range, so collapse the
        // range onto the single block that holds the centre instead.
        let min_x = min_x.floor() as i32;
        let max_x = (max_x.floor() as i32 - 1).max(min_x);
        let min_z = min_z.floor() as i32;
        let max_z = (max_z.floor() as i32 - 1).max(min_z);
        (x.clamp(min_x, max_x), z.clamp(min_z, max_z))
    }
}

impl World {
    /// Queues the initial border snapshot under its owner guard and delivers it unlocked.
    pub(crate) fn send_border_info(&self, player: &Player) {
        if player.client.java().is_none() {
            return;
        }
        let _effects = super::guard_effects::GuardEffects::new();
        self.worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .init_client(player);
    }
}

#[cfg(test)]
mod tests {
    use super::Worldborder;

    fn centered_border(diameter: f64) -> Worldborder {
        Worldborder::new(0.0, 0.0, diameter, 0, 5, 300)
    }

    /// `find_safe_location` only falls through to the `clamp_block` fallback
    /// because a zero-width border rejects every candidate it scans: `contains`
    /// reduces to `x >= center && x < center`, which no coordinate satisfies.
    #[test]
    fn a_zero_width_border_contains_no_block() {
        let border = centered_border(0.0);

        assert!(!border.contains_block(0, 0));
        assert!(
            (-32..=32).all(|x| (-32..=32).all(|z| !border.contains_block(x, z))),
            "a zero-width border should contain no block in the portal search area"
        );
    }

    /// `/worldborder set 0` is accepted, because the command's consumer is
    /// `BoundedNumArgumentConsumer::new().min(0.0)` and that bound is inclusive.
    #[test]
    fn clamp_block_handles_a_zero_width_border() {
        let border = centered_border(0.0);

        assert_eq!(border.clamp_block(0, 0), (0, 0));
        assert_eq!(border.clamp_block(100, -100), (0, 0));
    }

    /// A centre off the block grid puts both edges inside the same block, so the
    /// border spans no block boundary even though its diameter is not zero.
    #[test]
    fn clamp_block_handles_a_border_narrower_than_one_block() {
        let border = Worldborder::new(0.5, 0.5, 0.5, 0, 5, 300);

        assert_eq!(border.clamp_block(100, -100), (0, 0));
    }

    #[test]
    fn clamp_block_still_clamps_a_normal_border() {
        let border = centered_border(10.0);

        assert_eq!(border.clamp_block(100, 100), (4, 4));
        assert_eq!(border.clamp_block(-100, -100), (-5, -5));
        assert_eq!(border.clamp_block(2, -3), (2, -3));
    }
    #[test]
    fn random_teleport_clamps_both_axes_and_preserves_height() {
        use pumpkin_util::math::vector3::Vector3;
        let border = Worldborder::new(10.0, -20.0, 8.0, 0, 5, 15);
        let clamped = border.clamp_teleport_position(Vector3::new(-100.0, 73.25, 100.0));
        assert_eq!(clamped.x, 6.0);
        assert_eq!(clamped.y, 73.25);
        assert!(clamped.z < -16.0 && clamped.z > -16.00002);
    }
}

#[cfg(test)]
mod bounds_tests;
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
pub(crate) mod test_support;
