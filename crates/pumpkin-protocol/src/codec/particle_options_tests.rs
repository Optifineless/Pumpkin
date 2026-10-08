use super::*;
use pumpkin_data::item::Item;

fn fixture(
    particle: Particle,
    option: &ParticleOptions<'_>,
    expected: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = option.encode(&JavaMinecraftVersion::V_26_3)?;
    assert_eq!(bytes, expected, "{particle:?}");
    validate_options(VarInt(particle.to_id() as i32), expected)?;
    let mut input = expected;
    read_options(particle, &mut input)?;
    assert!(input.is_empty());
    Ok(())
}

#[test]
fn scalar_option_stream_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    // Handwritten from BlockParticleOption, DustParticleOptions, ColorParticleOption,
    // SculkChargeParticleOptions and the other scalar STREAM_CODECs (vanilla 26.3).
    let cases = [
        (
            Particle::Block,
            ParticleOptions::Block(300),
            vec![0xac, 0x02],
        ),
        (
            Particle::Dust,
            ParticleOptions::Dust {
                color: 0x123456,
                scale: 1.0,
            },
            vec![0, 0x12, 0x34, 0x56, 0x3f, 0x80, 0, 0],
        ),
        (
            Particle::DustColorTransition,
            ParticleOptions::DustColorTransition {
                from_color: 0x123456,
                to_color: 0xabcdef,
                scale: 0.5,
            },
            vec![0, 0x12, 0x34, 0x56, 0, 0xab, 0xcd, 0xef, 0x3f, 0, 0, 0],
        ),
        (
            Particle::EntityEffect,
            ParticleOptions::Color(0x12345678),
            vec![0x12, 0x34, 0x56, 0x78],
        ),
        (
            Particle::SculkCharge,
            ParticleOptions::SculkCharge(-0.5),
            vec![0xbf, 0, 0, 0],
        ),
        (
            Particle::Effect,
            ParticleOptions::Spell {
                color: 0x123456,
                power: 2.0,
            },
            vec![0, 0x12, 0x34, 0x56, 0x40, 0, 0, 0],
        ),
        (
            Particle::DragonBreath,
            ParticleOptions::Power(0.5),
            vec![0x3f, 0, 0, 0],
        ),
        (
            Particle::Shriek,
            ParticleOptions::Shriek(300),
            vec![0xac, 0x02],
        ),
        (
            Particle::Geyser,
            ParticleOptions::Geyser(3),
            vec![0, 0, 0, 3],
        ),
        (
            Particle::GeyserBase,
            ParticleOptions::GeyserBase {
                water_blocks: 3,
                burst_impulse_base: 1.0,
            },
            vec![0, 0, 0, 3, 0x3f, 0x80, 0, 0],
        ),
    ];
    for (particle, option, bytes) in cases {
        fixture(particle, &option, &bytes)?;
    }
    Ok(())
}

#[test]
fn item_option_is_a_template_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // ItemParticleOption.streamCodec: ItemStackTemplate, id BEFORE count, then patch.
    fixture(
        Particle::Item,
        &ParticleOptions::Item(&ItemStack::new(3, &Item::STONE)),
        &[1, 3, 0, 0],
    )
}

#[test]
fn vibration_position_source_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    fixture(
        Particle::Vibration,
        &ParticleOptions::Vibration {
            destination: VibrationDestination::Block(BlockPos::new(1, 2, 3)),
            arrival_in_ticks: 300,
        },
        &[0, 0, 0, 0, 0x40, 0, 0, 0x30, 2, 0xac, 0x02],
    )?;
    fixture(
        Particle::Vibration,
        &ParticleOptions::Vibration {
            destination: VibrationDestination::Entity {
                entity_id: 300,
                y_offset: 0.5,
            },
            arrival_in_ticks: 20,
        },
        &[1, 0xac, 0x02, 0x3f, 0, 0, 0, 20],
    )
}

#[test]
fn trail_option_has_three_doubles_color_and_duration() -> Result<(), Box<dyn std::error::Error>> {
    fixture(
        Particle::Trail,
        &ParticleOptions::Trail {
            target: Vector3::new(1.0, 2.0, -0.5),
            color: 0x123456,
            duration: 300,
        },
        &[
            0x3f, 0xf0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0xbf, 0xe0, 0, 0, 0, 0, 0, 0,
            0, 0x12, 0x34, 0x56, 0xac, 0x02,
        ],
    )
}

#[test]
fn truncated_and_extra_options_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ParticleOptions::Trail {
        target: Vector3::default(),
        color: 0,
        duration: 20,
    }
    .encode(&JavaMinecraftVersion::V_26_3)?;
    for length in 0..bytes.len() {
        assert!(
            validate_options(VarInt(Particle::Trail.to_id() as i32), &bytes[..length]).is_err()
        );
    }
    assert!(validate_options(VarInt(Particle::Flame.to_id() as i32), &[0]).is_err());
    Ok(())
}
