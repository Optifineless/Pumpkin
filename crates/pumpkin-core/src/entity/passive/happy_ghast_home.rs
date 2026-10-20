use super::HappyGhastEntity;
use crate::entity::EntityBase;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::chunk::ChunkHeightmapType;

#[derive(Clone, Copy)]
pub(super) struct Home {
    pos: BlockPos,
    radius: i32,
}
impl Default for Home {
    fn default() -> Self {
        Self {
            pos: BlockPos::new(0, 0, 0),
            radius: -1,
        }
    }
}

impl Home {
    pub(super) fn read(nbt: &NbtCompound) -> Self {
        let radius = nbt.get_int("home_radius").unwrap_or(-1);
        let pos = nbt
            .get_int_array("home_pos")
            .filter(|a| radius >= 0 && a.len() == 3)
            .map_or(BlockPos::new(0, 0, 0), |a| BlockPos::new(a[0], a[1], a[2]));
        Self { pos, radius }
    }
    pub(super) fn write(self, nbt: &mut NbtCompound) {
        if self.radius != -1 {
            nbt.put_int("home_radius", self.radius);
            nbt.put(
                "home_pos",
                pumpkin_nbt::tag::NbtTag::IntArray(vec![self.pos.0.x, self.pos.0.y, self.pos.0.z]),
            );
        }
    }
    fn contains(self, target: Vector3<f64>) -> bool {
        self.radius == -1
            || self.pos.dist_to_center_sqr(target.x, target.y, target.z)
                < f64::from(self.radius.wrapping_mul(self.radius))
    }
}

impl HappyGhastEntity {
    pub(super) fn wearing_body(&self) -> bool {
        !self
            .mob_entity
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&EquipmentSlot::BODY)
            .is_empty()
    }

    // HappyGhast.checkRestriction; integer block distance, unlike Vec3 candidate geometry.
    pub(super) fn check_restriction(&self) {
        let entity = self.get_entity();
        if entity.is_leashed() || entity.has_passengers() {
            return;
        }
        let radius = if self.wearing_body() { 32 } else { 64 };
        let home = self.home.load();
        let pos = entity.block_pos.load();
        let delta = Vector3::new(
            f64::from(home.pos.0.x) - f64::from(pos.0.x),
            f64::from(home.pos.0.y) - f64::from(pos.0.y),
            f64::from(home.pos.0.z) - f64::from(pos.0.z),
        );
        if home.radius == -1
            || delta.length_squared() >= f64::from(radius + 16).powi(2)
            || radius != home.radius
        {
            self.home.store(Home { pos, radius });
        }
    }

    fn choose_random_position(&self, center: Vector3<f64>) -> Vector3<f64> {
        center
            + Vector3::new(
                f64::from((self.random_float() * 2.0 - 1.0) * 16.0),
                f64::from((self.random_float() * 2.0 - 1.0) * 16.0),
                f64::from((self.random_float() * 2.0 - 1.0) * 16.0),
            )
    }

    fn is_good_target(&self, target: Vector3<f64>) -> bool {
        let world = self.get_entity().world.load();
        let pos = BlockPos::new(
            target.x.floor() as i32,
            target.y.floor() as i32,
            target.z.floor() as i32,
        );
        if !world
            .get_block_state_if_loaded(&pos)
            .is_some_and(pumpkin_data::BlockState::is_air)
        {
            return false;
        }
        for direction in [
            Vector3::new(1, 0, 0),
            Vector3::new(-1, 0, 0),
            Vector3::new(0, 1, 0),
            Vector3::new(0, -1, 0),
            Vector3::new(0, 0, 1),
            Vector3::new(0, 0, -1),
        ] {
            for distance in 1..16 {
                if world
                    .get_block_state_if_loaded(&BlockPos(pos.0 + direction * distance))
                    .is_some_and(|s| !s.is_air())
                {
                    return true;
                }
            }
        }
        false
    }

    // Ghast.RandomFloatAroundGoal.getSuitableFlyToPosition, including the last-candidate fallback.
    pub(super) fn suitable_fly_to_position(&self) -> Vector3<f64> {
        let entity = self.get_entity();
        let center = entity.pos.load();
        let mut result = None;
        for _ in 0..64 {
            let target = self.choose_random_position(center);
            result = self.home.load().contains(target).then_some(target);
            if let Some(target) = result
                && self.is_good_target(target)
            {
                return target;
            }
        }
        let mut result = result.unwrap_or_else(|| self.choose_random_position(center));
        let world = entity.world.load();
        let x = result.x.floor() as i32;
        let z = result.z.floor() as i32;
        // Pumpkin heightmaps store the highest occupied block; Java getHeight is first free Y.
        let height = world.get_heightmap_height(ChunkHeightmapType::MotionBlocking, x, z) + 1;
        if height < result.y.floor() as i32 && height > world.get_bottom_y() {
            result.y = center.y - (center.y - result.y).abs();
        }
        result
    }
}
