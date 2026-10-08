use crate::{
    VarInt,
    codec::item_stack_seralizer::ItemStackSerializer,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};
use pumpkin_data::{item_stack::ItemStack, particle::Particle};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    version::JavaMinecraftVersion,
};
use std::borrow::Cow;

// ScalableParticleOptionsBase.MIN_SCALE / MAX_SCALE (Java lines 8-9).
const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 4.0;

/// Payloads of vanilla particle option stream codecs, without the particle registry id.
pub enum ParticleOptions<'a> {
    Block(u32),
    Item(&'a ItemStack),
    Dust {
        color: i32,
        scale: f32,
    },
    DustColorTransition {
        from_color: i32,
        to_color: i32,
        scale: f32,
    },
    Color(i32),
    SculkCharge(f32),
    Vibration {
        destination: VibrationDestination,
        arrival_in_ticks: i32,
    },
    Trail {
        target: Vector3<f64>,
        color: i32,
        duration: i32,
    },
    Spell {
        color: i32,
        power: f32,
    },
    Power(f32),
    Shriek(i32),
    Geyser(i32),
    GeyserBase {
        water_blocks: i32,
        burst_impulse_base: f32,
    },
}

/// `PositionSource.STREAM_CODEC`'s block and entity destinations.
pub enum VibrationDestination {
    Block(BlockPos),
    Entity { entity_id: i32, y_offset: f32 },
}

impl ParticleOptions<'_> {
    /// Encodes a 26.3 option payload for a caller that supplies its matching Particle type.
    pub fn encode(&self, version: &JavaMinecraftVersion) -> Result<Vec<u8>, WritingError> {
        // Each arm mirrors the corresponding core.particles.*.STREAM_CODEC in vanilla 26.3.
        let mut bytes = Vec::new();
        match self {
            Self::Block(state) => bytes.write_var_int(&VarInt(*state as i32))?,
            Self::Item(stack) => ItemStackSerializer(Cow::Borrowed(stack))
                .write_template_with_version(&mut bytes, version)?,
            Self::Dust { color, scale } => {
                bytes.write_i32(*color)?;
                bytes.write_f32(scale.clamp(MIN_SCALE, MAX_SCALE))?;
            }
            Self::DustColorTransition {
                from_color,
                to_color,
                scale,
            } => {
                bytes.write_i32(*from_color)?;
                bytes.write_i32(*to_color)?;
                bytes.write_f32(scale.clamp(MIN_SCALE, MAX_SCALE))?;
            }
            Self::Color(color) | Self::Geyser(color) => bytes.write_i32(*color)?,
            Self::SculkCharge(value) | Self::Power(value) => bytes.write_f32(*value)?,
            Self::Vibration {
                destination,
                arrival_in_ticks,
            } => {
                match destination {
                    // PositionSourceType registers BLOCK first, ENTITY second (Java lines 10-11).
                    VibrationDestination::Block(pos) => {
                        bytes.write_var_int(&VarInt(0))?;
                        bytes.write_block_pos(pos, version)?;
                    }
                    VibrationDestination::Entity {
                        entity_id,
                        y_offset,
                    } => {
                        bytes.write_var_int(&VarInt(1))?;
                        bytes.write_var_int(&VarInt(*entity_id))?;
                        bytes.write_f32(*y_offset)?;
                    }
                }
                bytes.write_var_int(&VarInt(*arrival_in_ticks))?;
            }
            Self::Trail {
                target,
                color,
                duration,
            } => {
                bytes.write_f64_be(target.x)?;
                bytes.write_f64_be(target.y)?;
                bytes.write_f64_be(target.z)?;
                bytes.write_i32(*color)?;
                if *version >= JavaMinecraftVersion::V_1_21_4 {
                    bytes.write_var_int(&VarInt(*duration))?;
                }
            }
            Self::Spell { color, power } => {
                bytes.write_i32(*color)?;
                bytes.write_f32(*power)?;
            }
            Self::Shriek(delay) => bytes.write_var_int(&VarInt(*delay))?,
            Self::GeyserBase {
                water_blocks,
                burst_impulse_base,
            } => {
                bytes.write_i32(*water_blocks)?;
                bytes.write_f32(*burst_impulse_base)?;
            }
        }
        Ok(bytes)
    }
}

/// Consumes one 26.3 particle option payload, leaving subsequent packet fields untouched.
pub(crate) fn read_options(particle: Particle, input: &mut &[u8]) -> Result<(), ReadingError> {
    // ParticleTypes.STREAM_CODEC dispatches the option stream codec by registry id.
    match particle {
        Particle::Block
        | Particle::BlockMarker
        | Particle::FallingDust
        | Particle::DustPillar
        | Particle::BlockCrumble
        | Particle::Shriek => {
            input.get_var_int()?;
        }
        Particle::Item => {
            ItemStackSerializer::read_template_with_version(input, &JavaMinecraftVersion::V_26_3)?;
        }
        Particle::Dust
        | Particle::Effect
        | Particle::InstantEffect
        | Particle::GeyserBase
        | Particle::GeyserPoof => {
            input.get_i32()?;
            input.get_f32()?;
        }
        Particle::DustColorTransition => {
            input.get_i32()?;
            input.get_i32()?;
            input.get_f32()?;
        }
        Particle::EntityEffect
        | Particle::Flash
        | Particle::TintedLeaves
        | Particle::Geyser
        | Particle::GeyserPlume => {
            input.get_i32()?;
        }
        Particle::SculkCharge | Particle::DragonBreath => {
            input.get_f32()?;
        }
        Particle::Vibration => {
            match input.get_var_int()?.0 {
                0 => {
                    input.get_i64_be()?;
                }
                1 => {
                    input.get_var_int()?;
                    input.get_f32()?;
                }
                _ => {
                    return Err(ReadingError::Message(
                        "Unknown vibration destination".into(),
                    ));
                }
            }
            input.get_var_int()?;
        }
        Particle::Trail => {
            input.get_f64_be()?;
            input.get_f64_be()?;
            input.get_f64_be()?;
            input.get_i32()?;
            input.get_var_int()?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_options(id: VarInt, data: &[u8]) -> Result<(), WritingError> {
    let particle = u16::try_from(id.0)
        .ok()
        .and_then(Particle::from_id)
        .ok_or_else(|| WritingError::Message("Unknown particle type".into()))?;
    let mut input = data;
    read_options(particle, &mut input).map_err(|error| {
        WritingError::Message(format!("Invalid {particle:?} particle options: {error}"))
    })?;
    if !input.is_empty() {
        return Err(WritingError::Message(format!(
            "Extra {particle:?} particle option bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "particle_options_tests.rs"]
mod tests;
