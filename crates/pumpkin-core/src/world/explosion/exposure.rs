use super::{Explosion, World};
use crate::entity::Entity;
use pumpkin_data::BlockState;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::Arc;

impl Explosion {
    /// Collision-only visibility used by `ServerExplosion` and `FireworkRocketEntity`.
    pub(crate) fn ray_clear(world: &World, from: Vector3<f64>, to: Vector3<f64>) -> bool {
        World::traverse_blocks(from, to, |pos, _| {
            // ServerExplosion.getSeenPercent loading reads are clamped: unknown terrain occludes.
            (!world.level.is_chunk_loaded(&pos.chunk_position())
                || Self::clips_collision_shape(world.get_block_state(pos), pos, from, to))
            .then_some(())
        })
        .is_none()
    }

    pub(super) fn calculate_exposure(
        explosion_pos: &Vector3<f64>,
        entity: &Entity,
        world: &Arc<World>,
    ) -> f32 {
        let bbox = entity.bounding_box.load();

        let step_x = 1.0 / ((bbox.max.x - bbox.min.x) * 2.0 + 1.0);
        let step_y = 1.0 / ((bbox.max.y - bbox.min.y) * 2.0 + 1.0);
        let step_z = 1.0 / ((bbox.max.z - bbox.min.z) * 2.0 + 1.0);

        if step_x < 0.0 || step_y < 0.0 || step_z < 0.0 {
            return 0.0;
        }

        let offset_x = (1.0 - (1.0 / step_x).floor() * step_x) / 2.0;
        let offset_z = (1.0 - (1.0 / step_z).floor() * step_z) / 2.0;

        let mut visible_points = 0;
        let mut total_points = 0;

        let mut k = 0.0;
        while k <= 1.0 {
            let mut l = 0.0;
            while l <= 1.0 {
                let mut m = 0.0;
                while m <= 1.0 {
                    let n = bbox.min.x + (bbox.max.x - bbox.min.x) * k;
                    let o = bbox.min.y + (bbox.max.y - bbox.min.y) * l;
                    let p = bbox.min.z + (bbox.max.z - bbox.min.z) * m;

                    let vec3d = Vector3::new(n + offset_x, o, p + offset_z);

                    if Self::ray_clear(world, vec3d, *explosion_pos) {
                        visible_points += 1;
                    }

                    total_points += 1;
                    m += step_z;
                }
                l += step_y;
            }
            k += step_x;
        }

        if total_points == 0 {
            return 0.0;
        }

        visible_points as f32 / total_points as f32
    }

    // Vanilla VoxelShape.clip / AABB.clip, using the original ray endpoints.
    pub(super) fn clips_collision_shape(
        state: &BlockState,
        pos: &BlockPos,
        from: Vector3<f64>,
        to: Vector3<f64>,
    ) -> bool {
        crate::entity::projectile::clip::clip_block(state, pos, from, to).is_some()
    }
}
