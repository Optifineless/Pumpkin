// bucket variant STREAM_CODEC values are enum ids, not strings.
use pumpkin_data::data_component_impl::{
    AxolotlVariantImpl, SalmonSizeImpl, TropicalFishBaseColorImpl, TropicalFishPatternColorImpl,
    TropicalFishPatternImpl,
};
use pumpkin_data::dye_color::DyeColor;

use super::data_component::DataComponentCodec;
use crate::{
    VarInt,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};

macro_rules! enum_codec {
    ($ty:ty, $normalize:expr) => {
        impl DataComponentCodec<Self> for $ty {
            fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
                let id = self
                    .variant_id()
                    .ok_or_else(|| WritingError::Message("Invalid bucket variant".into()))?;
                seq.write_var_int(&VarInt(id))
            }
            fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
                Self::from_variant_id(($normalize)(seq.get_var_int()?.0))
                    .ok_or_else(|| ReadingError::Message("Invalid bucket variant id".into()))
            }
        }
    };
}
// Axolotl.Variant uses ByIdMap.ZERO, Salmon.Variant uses CLAMP, Pattern uses sparse KOB.
enum_codec!(
    AxolotlVariantImpl,
    |id| if (0..AxolotlVariantImpl::NAMES.len() as i32).contains(&id) {
        id
    } else {
        0
    }
);
enum_codec!(SalmonSizeImpl, |id: i32| id
    .clamp(0, SalmonSizeImpl::NAMES.len() as i32 - 1));
enum_codec!(
    TropicalFishPatternImpl,
    |id| if TropicalFishPatternImpl::from_variant_id(id).is_some() {
        id
    } else {
        0
    }
);

macro_rules! color_codec {
    ($ty:ty) => {
        impl DataComponentCodec<Self> for $ty {
            fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
                let color = DyeColor::by_name(&self.value)
                    .ok_or_else(|| WritingError::Message("Invalid bucket color".into()))?;
                seq.write_var_int(&VarInt(i32::from(color.id())))
            }
            fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
                let color = u8::try_from(seq.get_var_int()?.0)
                    .ok()
                    .and_then(DyeColor::by_id)
                    .unwrap_or(DyeColor::White);
                Ok(Self {
                    value: color.name().into(),
                })
            }
        }
    };
}
color_codec!(TropicalFishBaseColorImpl);
color_codec!(TropicalFishPatternColorImpl);
#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::data_component_impl::BucketEntityDataImpl;

    #[test]
    #[expect(clippy::unwrap_used, reason = "Vanilla enum wire fixtures")]
    fn bucket_variants_use_enum_ids_on_the_wire() {
        let mut bytes = Vec::new();
        AxolotlVariantImpl {
            value: "blue".into(),
        }
        .serialize(&mut bytes)
        .unwrap();
        assert_eq!(bytes, [4]);
        assert_eq!(
            AxolotlVariantImpl::deserialize(&mut &[4][..])
                .unwrap()
                .value,
            "blue"
        );
        bytes.clear();
        SalmonSizeImpl {
            value: "large".into(),
        }
        .serialize(&mut bytes)
        .unwrap();
        assert_eq!(bytes, [2]);
        bytes.clear();
        TropicalFishPatternImpl {
            value: "clayfish".into(),
        }
        .serialize(&mut bytes)
        .unwrap();
        assert_eq!(bytes, [0x81, 0x0a]); // Pattern.LARGE base 1, index 5, packed id 1281
        bytes.clear();
        TropicalFishBaseColorImpl {
            value: "blue".into(),
        }
        .serialize(&mut bytes)
        .unwrap();
        assert_eq!(bytes, [11]);
    }

    #[test]
    #[expect(clippy::unwrap_used, reason = "Vanilla ByIdMap fallback fixtures")]
    fn bucket_variant_decoding_keeps_vanilla_enum_fallbacks() {
        assert_eq!(
            AxolotlVariantImpl::deserialize(&mut &[127][..])
                .unwrap()
                .value,
            "lucy"
        );
        assert_eq!(
            SalmonSizeImpl::deserialize(&mut &[127][..]).unwrap().value,
            "large"
        );
        assert_eq!(
            TropicalFishPatternImpl::deserialize(&mut &[127][..])
                .unwrap()
                .value,
            "kob"
        );
        assert_eq!(
            TropicalFishPatternColorImpl::deserialize(&mut &[127][..])
                .unwrap()
                .value,
            "white"
        );
    }

    #[test]
    #[expect(clippy::unwrap_used, reason = "Known vanilla network compound bytes")]
    fn bucket_entity_data_codec_preserves_the_compound_payload() {
        // CustomData.STREAM_CODEC: unnamed compound containing Health=7.0f.
        let bytes = [
            10, 5, 0, 6, b'H', b'e', b'a', b'l', b't', b'h', 0x40, 0xe0, 0, 0, 0,
        ];
        let data = BucketEntityDataImpl::deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(
            data.nbt.as_ref().and_then(|tag| tag.get_float("Health")),
            Some(7.0)
        );
        let mut encoded = Vec::new();
        data.serialize(&mut encoded).unwrap();
        assert_eq!(encoded, bytes);
    }
}
