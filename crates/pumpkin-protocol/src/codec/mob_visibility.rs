use super::{DataComponentCodec, deserialize_idset, serialize_idset};
use crate::ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError};
use pumpkin_data::data_component_impl::MobVisibilityImpl;

impl DataComponentCodec<Self> for MobVisibilityImpl {
    // MobVisibility.STREAM_CODEC: entity holder set followed by a float.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_idset(&self.targeting_entity_types, seq)?;
        seq.write_f32(self.visibility)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        Ok(Self {
            targeting_entity_types: deserialize_idset(seq)?,
            visibility: seq.get_f32()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::data_component::{deserialize, serialize},
        codec::var_int::VarInt,
    };
    use pumpkin_data::data_component::DataComponent;
    use std::io::Cursor;

    #[test]
    fn avoidance_followup_mob_visibility_stream_matches_vanilla_holder_set_and_float() {
        // ByteBufCodecs.holderSet: zero selects a tag, followed by UTF-8; FLOAT is big-endian.
        let bytes = b"\x00\x13minecraft:skeletons\x3f\x00\x00\x00";
        let mut input = Cursor::new(bytes);
        let value = deserialize(DataComponent::MobVisibility, &mut input).unwrap();
        assert_eq!(
            value
                .as_any()
                .downcast_ref::<MobVisibilityImpl>()
                .unwrap()
                .visibility,
            0.5
        );
        let mut output = Vec::new();
        serialize(DataComponent::MobVisibility, value.as_ref(), &mut output).unwrap();
        assert_eq!(output.as_slice(), bytes);
        let mut oversized = Vec::new();
        oversized.write_var_int(&VarInt(4098)).unwrap();
        assert!(MobVisibilityImpl::deserialize(&mut Cursor::new(oversized)).is_err());
    }
}
