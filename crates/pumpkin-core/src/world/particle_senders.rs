use pumpkin_data::particle::Particle;
use pumpkin_protocol::{codec::particle_options::ParticleOptions, java::client::play::CParticle};
use pumpkin_util::{math::vector3::Vector3, version::JavaMinecraftVersion};

use super::World;
use crate::net::ClientPlatform;

impl World {
    /// Sends matching typed options with both flags false, encoded for each Java client's version.
    pub fn spawn_particle_with_options(
        &self,
        position: Vector3<f64>,
        offset: Vector3<f32>,
        max_speed: f32,
        particle_count: i32,
        particle: Particle,
        options: &ParticleOptions<'_>,
    ) {
        // ServerLevel.sendParticles preserves the ParticleOptions, rather than just the type ID.
        for player in self.players.load().iter() {
            let ClientPlatform::Java(client) = player.client.as_ref() else {
                player.spawn_particle(position, offset, max_speed, particle_count, particle);
                continue;
            };
            let version = client.version.load();
            if particle == Particle::Trail && version < JavaMinecraftVersion::V_1_21_2 {
                continue;
            }
            let Ok(data) = options.encode(&version) else {
                continue;
            };
            player.try_send_client_packet(&CParticle::new(
                false,
                false,
                position,
                offset,
                max_speed,
                particle_count,
                i32::from(particle.to_id()).into(),
                &data,
            ));
        }
    }
}
