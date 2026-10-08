use super::*;
use pumpkin_data::particle::Particle;

#[test]
fn empty_options_never_reach_the_client() {
    for particle in [
        Particle::Block,
        Particle::Item,
        Particle::Dust,
        Particle::DustColorTransition,
        Particle::EntityEffect,
        Particle::SculkCharge,
        Particle::Vibration,
        Particle::Trail,
    ] {
        let packet = CParticle::new(
            false,
            false,
            Vector3::default(),
            Vector3::default(),
            0.0,
            1,
            VarInt(particle.to_id() as i32),
            &[],
        );
        let mut bytes = Vec::new();
        assert!(
            packet
                .write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)
                .is_err(),
            "empty {particle:?} options must be rejected"
        );
        assert!(bytes.is_empty());
    }
}

#[test]
fn particle_packet_matches_vanilla_26_3_field_order() -> Result<(), Box<dyn std::error::Error>> {
    // ClientboundLevelParticlesPacket.STREAM_CODEC, with a ColorParticleOption.
    let packet = CParticle::new(
        false,
        true,
        Vector3::new(1.0, 2.0, -0.5),
        Vector3::new(0.5, 1.0, -1.0),
        0.25,
        300,
        VarInt(Particle::EntityEffect.to_id() as i32),
        &[0x12, 0x34, 0x56, 0x78],
    );
    let mut bytes = Vec::new();
    packet.write_packet_data(&mut bytes, &JavaMinecraftVersion::V_26_3)?;
    let mut expected = vec![Particle::EntityEffect.to_id() as u8];
    expected.extend_from_slice(&[
        0x12, 0x34, 0x56, 0x78, 1, 0, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0,
        0xbf, 0xe0, 0, 0, 0, 0, 0, 0, 0x3f, 0, 0, 0, 0x3f, 0x80, 0, 0, 0xbf, 0x80, 0, 0, 0x3e,
        0x80, 0, 0, 0x3e, 0x80, 0, 0, 0x3e, 0x80, 0, 0, 0xac, 0x02, 0,
    ]);
    assert_eq!(bytes, expected);
    // Decode independently of CParticle::read, following vanilla's 26.3 field sequence.
    let mut input = bytes.as_slice();
    assert_eq!(
        input.get_var_int()?.0,
        i32::from(Particle::EntityEffect.to_id())
    );
    assert_eq!(input.get_i32()?, 0x12345678);
    assert!(input.get_bool()?);
    assert!(!input.get_bool()?);
    assert_eq!(
        [
            input.get_f64_be()?,
            input.get_f64_be()?,
            input.get_f64_be()?
        ],
        [1.0, 2.0, -0.5]
    );
    assert_eq!(
        [input.get_f32()?, input.get_f32()?, input.get_f32()?],
        [0.5, 1.0, -1.0]
    );
    assert_eq!(
        [input.get_f32()?, input.get_f32()?, input.get_f32()?],
        [0.25, 0.25, 0.25]
    );
    assert_eq!(input.get_var_int()?.0, 300);
    assert_eq!(input.get_var_int()?.0, 0);
    assert!(input.is_empty());
    Ok(())
}
