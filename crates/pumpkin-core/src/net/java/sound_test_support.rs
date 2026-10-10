use bytes::Bytes;
use pumpkin_data::packet::clientbound::play::SOUND;
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::vector3::Vector3;

#[derive(Clone, Debug, PartialEq)]
pub struct SoundPacket {
    pub sound_id: u16,
    pub category: i32,
    pub position: Vector3<i32>,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}

/// Decodes registered sounds while retaining the caller's complete packet capture.
pub fn decode_sounds(packets: &[Bytes]) -> Vec<SoundPacket> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if data.get_var_int().unwrap().0 != SOUND.0 {
                return None;
            }
            // The 26.3 registry holder reserves zero for an inline sound event.
            let holder = data.get_var_int().unwrap().0;
            assert!(holder > 0, "sound fixture requires a registered event");
            let sound = SoundPacket {
                sound_id: u16::try_from(holder - 1).unwrap(),
                category: data.get_var_int().unwrap().0,
                position: Vector3::new(
                    data.get_i32_be().unwrap(),
                    data.get_i32_be().unwrap(),
                    data.get_i32_be().unwrap(),
                ),
                volume: data.get_f32_be().unwrap(),
                pitch: data.get_f32_be().unwrap(),
                seed: data.get_i64_be().unwrap(),
            };
            assert!(data.is_empty());
            Some(sound)
        })
        .collect()
}
