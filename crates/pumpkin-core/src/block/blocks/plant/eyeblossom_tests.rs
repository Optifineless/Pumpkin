use super::*;
use pumpkin_protocol::{ClientPacket, java::client::play::CParticle};
use pumpkin_util::version::JavaMinecraftVersion;

struct ZeroRng;
impl rand::TryRng for ZeroRng {
    type Error = std::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(0)
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(0)
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        dst.fill(0);
        Ok(())
    }
}

#[test]
fn eyeblossom_trail_matches_vanilla_bytes() {
    // Zero random draws at (0,64,0): target=(.25,65,.25), duration=10.
    let pos = BlockPos::new(0, 64, 0);
    let version = JavaMinecraftVersion::V_26_3;
    for (block, color) in [
        (&Block::OPEN_EYEBLOSSOM, [0, 0xfc, 0x78, 0x12]),
        (&Block::CLOSED_EYEBLOSSOM, [0, 0x5f, 0x5f, 0x5f]),
    ] {
        let options = transform_particle(block, &pos, &mut ZeroRng);
        let data = options.encode(&version).unwrap();
        let mut expected = vec![
            0x3f, 0xd0, 0, 0, 0, 0, 0, 0, // .25
            0x40, 0x50, 0x40, 0, 0, 0, 0, 0, // 65
            0x3f, 0xd0, 0, 0, 0, 0, 0, 0,
        ];
        expected.extend_from_slice(&color);
        expected.push(10);
        assert_eq!(data, expected);
        let packet = CParticle::new(
            false,
            false,
            pos.to_centered_f64(),
            Vector3::default(),
            0.0,
            1,
            i32::from(Particle::Trail.to_id()).into(),
            &data,
        );
        let mut bytes = Vec::new();
        packet.write_packet_data(&mut bytes, &version).unwrap();
        // Options precede the two false flags in ClientboundLevelParticlesPacket.STREAM_CODEC.
        assert_eq!(&bytes[1..30], &expected);
        assert_eq!(&bytes[30..32], &[0, 0]);
        assert_eq!(&bytes[bytes.len() - 2..], &[1, 0]); // count and randomization
    }
}
