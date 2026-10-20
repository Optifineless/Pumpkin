use super::DataComponentCodec;
use crate::ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError};
use pumpkin_data::data_component_impl::{BrewingFuelImpl, BrewingFuelProvider};
use pumpkin_util::identifier::Identifier;
use std::borrow::Cow;

impl DataComponentCodec<Self> for BrewingFuelImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // BrewingFuel.STREAM_CODEC composes ResolvableInt then ResolvableFloat.
        match &self.uses {
            BrewingFuelProvider::Constant(uses) => {
                seq.write_bool(true)?;
                seq.write_i32(*uses)?;
            }
            BrewingFuelProvider::Id(id) => write_reference(id, seq)?,
        }
        match &self.speed_multiplier {
            BrewingFuelProvider::Constant(speed) => {
                seq.write_bool(true)?;
                seq.write_f32(*speed)
            }
            BrewingFuelProvider::Id(id) => write_reference(id, seq),
        }
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        // ByteBufCodecs.either selects the left (constant) codec for a true boolean.
        let uses = if seq.get_bool()? {
            BrewingFuelProvider::Constant(seq.get_i32()?)
        } else {
            BrewingFuelProvider::Id(read_reference(seq)?)
        };
        let speed_multiplier = if seq.get_bool()? {
            BrewingFuelProvider::Constant(seq.get_f32()?)
        } else {
            BrewingFuelProvider::Id(read_reference(seq)?)
        };
        Ok(Self {
            uses,
            speed_multiplier,
        })
    }
}

fn read_reference(seq: &mut impl NetworkReadExt) -> Result<Cow<'static, str>, ReadingError> {
    let id = Identifier::parse(&seq.get_str()?)
        .map_err(|_| ReadingError::Message("Invalid brewing fuel provider identifier".into()))?;
    Ok(Cow::Owned(id.to_string()))
}

fn write_reference(id: &str, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
    let id = Identifier::parse(id)
        .map_err(|_| WritingError::Message("Invalid brewing fuel provider identifier".into()))?;
    seq.write_bool(false)?;
    seq.write_string(&id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::data_component::{deserialize, serialize};
    use pumpkin_data::{data_component::DataComponent, data_component_impl::DataComponentImpl};

    fn assert_wire(fuel: &BrewingFuelImpl, expected: &[u8]) {
        let mut encoded = Vec::new();
        serialize(DataComponent::BrewingFuel, fuel, &mut encoded).unwrap();
        assert_eq!(encoded, expected);
        let mut input = expected;
        let decoded = deserialize(DataComponent::BrewingFuel, &mut input).unwrap();
        assert!(input.is_empty());
        assert!(fuel.equal(decoded.as_ref()));
    }

    #[test]
    fn b1019_g02_brewing_fuel_registered_codec_matches_vanilla_constants() {
        assert_wire(
            &BrewingFuelImpl {
                uses: BrewingFuelProvider::Constant(7),
                speed_multiplier: BrewingFuelProvider::Constant(2.5),
            },
            &[1, 0, 0, 0, 7, 1, 0x40, 0x20, 0, 0],
        );
        // Netty ByteBuf.readBoolean, used by either, accepts any nonzero byte.
        let mut bytes: &[u8] = &[2, 0, 0, 0, 7, 3, 0x40, 0x20, 0, 0];
        let decoded = deserialize(DataComponent::BrewingFuel, &mut bytes).unwrap();
        let fuel = decoded.as_any().downcast_ref::<BrewingFuelImpl>().unwrap();
        assert_eq!(fuel.uses, BrewingFuelProvider::Constant(7));
        assert_eq!(fuel.speed_multiplier, BrewingFuelProvider::Constant(2.5));
    }

    #[test]
    fn b1019_g02_brewing_fuel_registered_codec_matches_vanilla_references() {
        assert_wire(
            &BrewingFuelImpl {
                uses: BrewingFuelProvider::Id(Cow::Borrowed("test:uses")),
                speed_multiplier: BrewingFuelProvider::Id(Cow::Borrowed("test:speed")),
            },
            b"\x00\x09test:uses\x00\x0atest:speed",
        );
        assert_wire(
            &BrewingFuelImpl::default(),
            b"\x00\x1eminecraft:brewing/uses_default\x00\x1fminecraft:brewing/speed_default",
        );
    }

    #[test]
    fn b1019_g02_brewing_fuel_registered_codec_rejects_invalid_or_incomplete_data() {
        let valid = b"\x00\x09test:uses\x00\x0atest:speed";
        for end in 0..valid.len() {
            assert!(
                deserialize(DataComponent::BrewingFuel, &mut &valid[..end]).is_err(),
                "truncated brewing fuel payload at {end} must fail"
            );
        }
        let invalid = b"\x00\x03A:a\x01\x3f\x80\x00\x00";
        assert!(deserialize(DataComponent::BrewingFuel, &mut &invalid[..]).is_err());
    }
}
