use super::DataComponentImpl;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VillagerFoodImpl {
    pub nutrition: i32,
}

impl VillagerFoodImpl {
    // VillagerFood.CODEC uses Codec.INT's Number.intValue, then POSITIVE_INT.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let nutrition = match *data.extract_compound()?.get("nutrition")? {
            NbtTag::Byte(value) => i32::from(value),
            NbtTag::Short(value) => i32::from(value),
            NbtTag::Int(value) => value,
            NbtTag::Long(value) => value as i32,
            NbtTag::Float(value) => value as i32,
            NbtTag::Double(value) => value as i32,
            _ => return None,
        };
        (nutrition > 0).then_some(Self { nutrition })
    }
}

impl DataComponentImpl for VillagerFoodImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put_int("nutrition", self.nutrition);
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> i32 {
        super::instrument::hash::hash_tag(&self.write_data()) as i32
    }

    default_impl!(VillagerFood);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_component::DataComponent;

    #[test]
    fn villager_s2_required_numeric_nutrition_and_canonical_hash() {
        for (numeric, expected) in [
            (NbtTag::Byte(7), 7),
            (NbtTag::Short(300), 300),
            (NbtTag::Int(i32::MAX), i32::MAX),
            (NbtTag::Long(4_294_967_303), 7),
            (NbtTag::Float(7.9), 7),
            (NbtTag::Double(4.9), 4),
            (NbtTag::Double(f64::INFINITY), i32::MAX),
        ] {
            let mut compound = NbtCompound::new();
            compound.put("nutrition", numeric);
            let decoded = VillagerFoodImpl::read_data(&NbtTag::Compound(compound));
            assert_eq!(
                decoded,
                Some(VillagerFoodImpl {
                    nutrition: expected
                })
            );
        }
        for numeric in [
            NbtTag::Int(0),
            NbtTag::Int(-1),
            NbtTag::Long(4_294_967_296),
            NbtTag::Float(f32::NAN),
            NbtTag::Double(f64::NEG_INFINITY),
            NbtTag::String("7".into()),
        ] {
            let mut compound = NbtCompound::new();
            compound.put("nutrition", numeric);
            assert!(VillagerFoodImpl::read_data(&NbtTag::Compound(compound)).is_none());
        }
        assert!(VillagerFoodImpl::read_data(&NbtTag::Compound(NbtCompound::new())).is_none());
        assert!(VillagerFoodImpl::read_data(&NbtTag::Int(7)).is_none());
        for (nutrition, hash) in [(7, -539_925_475), (4, -964_794_316)] {
            let value = VillagerFoodImpl { nutrition };
            assert_eq!(value.get_hash(), hash);
            let encoded = value.write_data();
            assert_eq!(
                encoded
                    .extract_compound()
                    .and_then(|c| c.get_int("nutrition")),
                Some(nutrition)
            );
            assert!(
                super::super::read_data(DataComponent::VillagerFood, &encoded)
                    .is_some_and(|decoded| decoded.equal(&value))
            );
        }
    }
}
