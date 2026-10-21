// Last verified for v2169

use pumpkin_macros::packet;
use pumpkin_util::math::vector3::Vector3;

use crate::{codec::var_int::VarInt, serial::PacketWrite};

/// Broadcasts visual or auditory level-wide events, such as particles or block breaking.
#[derive(PacketWrite)]
#[packet(25)]
pub struct CLevelEvent {
    /// Identifier corresponding to the specific level event action.
    pub event_id: VarInt,
    /// World coordinates where the event originates or particle effect is centered.
    pub position: Vector3<f32>,
    /// Type-specific auxiliary parameter associated with the event (e.g. block runtime ID).
    pub data: VarInt,
}

/// Identifiers for common level-wide events and particle effects.
#[repr(i32)]
pub enum LevelEvent {
    // There are hundreds of these, adding only what we need for now
    /// Emits block break particles at the target position.
    ParticlesDestroyBlock = 2001,
    /// Emits copper waxing particles (Cloudburst v431, retained in v2193).
    ParticlesWaxOn = 2030,
    /// Emits copper unwaxing particles.
    ParticlesWaxOff = 2031,
    /// Emits copper scraping particles.
    ParticlesScrape = 2032,
    /// Initiates a block cracking animation.
    BlockStartBreak = 3600,
    /// Cancels or completes a block cracking animation.
    BlockStopBreak = 3601,
    /// Advances the crack stage of a block cracking animation.
    BlockUpdateBreak = 3602,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b1020d_g10_wax_scrape_level_event_packets() {
        // Cloudburst v2193 inherits v431's 2031/2032 IDs and v291's signed event/data codec.
        for (event, expected) in [
            (
                LevelEvent::ParticlesWaxOff,
                [
                    0xde, 0x1f, 0, 0, 0xc0, 0x3f, 0, 0, 0x81, 0x42, 0, 0, 0x20, 0xc0, 0,
                ],
            ),
            (
                LevelEvent::ParticlesScrape,
                [
                    0xe0, 0x1f, 0, 0, 0xc0, 0x3f, 0, 0, 0x81, 0x42, 0, 0, 0x20, 0xc0, 0,
                ],
            ),
        ] {
            let packet = CLevelEvent {
                event_id: VarInt(event as i32),
                position: Vector3::new(1.5, 64.5, -2.5),
                data: VarInt(0),
            };
            let mut actual = Vec::new();
            packet.write(&mut actual).unwrap();
            assert_eq!(actual, expected);
        }
    }
}
