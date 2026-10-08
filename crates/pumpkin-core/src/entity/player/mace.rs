use super::Player;
use crate::entity::EntityBase;
use pumpkin_data::particle::Particle;
use pumpkin_protocol::{
    codec::var_int::VarInt, java::client::play::CParticle, ser::NetworkWriteExt,
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

impl Player {
    // ServerPlayer.checkFallDamage emits block particles before ordinary landing damage.
    pub(crate) fn check_mace_landing_particles(&self, distance: f32) {
        if distance <= 0.0
            || !self
                .spawn_extra_particles_on_fall
                .swap(false, Ordering::Relaxed)
        {
            return;
        }
        let entity = self.get_entity();
        let world = self.world();
        let pos = entity.get_pos_with_y_offset(0.2).0;
        let state = world.get_block_state(&pos);
        let mut data = Vec::new();
        if data
            .write_var_int(&VarInt(i32::from(state.id.as_u16())))
            .is_err()
        {
            return;
        }
        let centered = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 1.0,
            f64::from(pos.0.z) + 0.5,
        );
        world.broadcast_packet_all(&CParticle::new(
            false,
            false,
            centered,
            Vector3::new(0.3, 0.3, 0.3),
            0.15,
            (50.0 * distance).clamp(0.0, 200.0) as i32,
            VarInt(Particle::Block.to_id() as i32),
            &data,
        ));
    }
}
