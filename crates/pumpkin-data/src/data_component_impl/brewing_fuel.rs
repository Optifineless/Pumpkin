use super::DataComponentImpl;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::identifier::Identifier;
use std::{
    borrow::Cow,
    hash::{Hash, Hasher},
};

/// A registry reference or constant used by brewing fuel's resolvable providers.
#[derive(Clone, Debug)]
pub enum BrewingFuelProvider<T> {
    Id(Cow<'static, str>),
    Constant(T),
}

impl PartialEq for BrewingFuelProvider<i32> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Id(left), Self::Id(right)) => left == right,
            (Self::Constant(left), Self::Constant(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for BrewingFuelProvider<i32> {}

impl Hash for BrewingFuelProvider<i32> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Id(id) => {
                0u8.hash(state);
                id.hash(state);
            }
            Self::Constant(value) => {
                1u8.hash(state);
                value.hash(state);
            }
        }
    }
}

impl PartialEq for BrewingFuelProvider<f32> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Id(left), Self::Id(right)) => left == right,
            (Self::Constant(left), Self::Constant(right)) => left.to_bits() == right.to_bits(),
            _ => false,
        }
    }
}

impl Eq for BrewingFuelProvider<f32> {}

impl Hash for BrewingFuelProvider<f32> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Id(id) => {
                0u8.hash(state);
                id.hash(state);
            }
            Self::Constant(value) => {
                1u8.hash(state);
                value.to_bits().hash(state);
            }
        }
    }
}

impl BrewingFuelProvider<i32> {
    fn read_data(tag: &NbtTag) -> Option<Self> {
        Some(match tag {
            NbtTag::String(id) => Self::Id(Cow::Owned(Identifier::parse(id).ok()?.to_string())),
            _ => Self::Constant(match tag {
                NbtTag::Byte(value) => i32::from(*value),
                NbtTag::Short(value) => i32::from(*value),
                NbtTag::Int(value) => *value,
                NbtTag::Long(value) => *value as i32,
                NbtTag::Float(value) => *value as i32,
                NbtTag::Double(value) => *value as i32,
                _ => return None,
            }),
        })
    }

    #[must_use]
    pub fn to_nbt(&self) -> NbtTag {
        match self {
            Self::Id(id) => reference_nbt(id),
            Self::Constant(value) => NbtTag::Int(*value),
        }
    }
}

impl BrewingFuelProvider<f32> {
    fn read_data(tag: &NbtTag) -> Option<Self> {
        Some(match tag {
            NbtTag::String(id) => Self::Id(Cow::Owned(Identifier::parse(id).ok()?.to_string())),
            _ => Self::Constant(tag.as_numeric_float()?),
        })
    }

    #[must_use]
    pub fn to_nbt(&self) -> NbtTag {
        match self {
            Self::Id(id) => reference_nbt(id),
            Self::Constant(value) => NbtTag::Float(*value),
        }
    }
}

fn reference_nbt(id: &str) -> NbtTag {
    // ResourceKey.CODEC writes the canonical namespaced Identifier.
    NbtTag::String(
        Identifier::parse(id)
            .map_or_else(|_| id.to_string(), |identifier| identifier.to_string())
            .into(),
    )
}

/// The uses and speed context providers carried by brewing fuel.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BrewingFuelImpl {
    pub uses: BrewingFuelProvider<i32>,
    pub speed_multiplier: BrewingFuelProvider<f32>,
}

impl Default for BrewingFuelImpl {
    fn default() -> Self {
        Self {
            uses: BrewingFuelProvider::Id(Cow::Borrowed("minecraft:brewing/uses_default")),
            speed_multiplier: BrewingFuelProvider::Id(Cow::Borrowed(
                "minecraft:brewing/speed_default",
            )),
        }
    }
}

impl BrewingFuelImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        // Older fork builds represented this component as an empty marker.
        if matches!(data, NbtTag::End) {
            return Some(Self::default());
        }
        let compound = data.extract_compound()?;
        if compound.child_tags.is_empty() {
            return Some(Self::default());
        }
        // BrewingFuel.CODEC requires both provider fields for a non-marker payload.
        Some(Self {
            uses: BrewingFuelProvider::<i32>::read_data(compound.get("uses")?)?,
            speed_multiplier: BrewingFuelProvider::<f32>::read_data(
                compound.get("speed_multiplier")?,
            )?,
        })
    }
}

impl DataComponentImpl for BrewingFuelImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("uses", self.uses.to_nbt());
        compound.put("speed_multiplier", self.speed_multiplier.to_nbt());
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> i32 {
        // BrewingFuel.CODEC uses RegistryOps<HashOps>, including provider payloads.
        super::instrument::hash::hash_tag(&self.write_data()) as i32
    }

    default_impl!(BrewingFuel);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b1019_g02_brewing_fuel_preserves_provider_payloads() {
        for fuel in [
            BrewingFuelImpl::default(),
            BrewingFuelImpl {
                uses: BrewingFuelProvider::Constant(7),
                speed_multiplier: BrewingFuelProvider::Constant(2.5),
            },
            BrewingFuelImpl {
                uses: BrewingFuelProvider::Id(Cow::Borrowed("test:brewing_uses")),
                speed_multiplier: BrewingFuelProvider::Id(Cow::Borrowed("test:brewing_speed")),
            },
        ] {
            let restored = BrewingFuelImpl::read_data(&fuel.write_data()).unwrap();
            assert_eq!(restored, fuel);
            assert_eq!(restored.get_hash(), fuel.get_hash());
            assert_ne!(
                fuel.get_hash(),
                0,
                "fuel payload cannot retain the marker hash"
            );
        }
        let first = BrewingFuelImpl {
            uses: BrewingFuelProvider::Constant(7),
            speed_multiplier: BrewingFuelProvider::Constant(2.5),
        };
        let second = BrewingFuelImpl {
            uses: BrewingFuelProvider::Constant(9),
            ..first.clone()
        };
        assert_ne!(first.get_hash(), second.get_hash());
        let numeric_reference = BrewingFuelImpl {
            uses: BrewingFuelProvider::Id(Cow::Borrowed("7")),
            ..first.clone()
        };
        let canonical_reference = BrewingFuelImpl {
            uses: BrewingFuelProvider::Id(Cow::Borrowed("minecraft:7")),
            ..first
        };
        assert_eq!(
            numeric_reference.uses.to_nbt(),
            NbtTag::String("minecraft:7".into())
        );
        assert_eq!(numeric_reference.get_hash(), canonical_reference.get_hash());
        let nan_speed = BrewingFuelImpl {
            uses: BrewingFuelProvider::Constant(7),
            speed_multiplier: BrewingFuelProvider::Constant(f32::from_bits(0x7fc0_0001)),
        };
        assert_eq!(nan_speed, nan_speed.clone());
        assert_eq!(
            BrewingFuelImpl::read_data(&nan_speed.write_data()),
            Some(nan_speed.clone())
        );
        let positive_zero = BrewingFuelImpl {
            speed_multiplier: BrewingFuelProvider::Constant(0.0),
            ..nan_speed.clone()
        };
        let negative_zero = BrewingFuelImpl {
            speed_multiplier: BrewingFuelProvider::Constant(-0.0),
            ..nan_speed.clone()
        };
        assert_ne!(positive_zero, negative_zero);
        assert_ne!(positive_zero.get_hash(), negative_zero.get_hash());
        let mut fuels = std::collections::HashSet::new();
        assert!(fuels.insert(nan_speed.clone()));
        assert!(!fuels.insert(nan_speed));
        assert!(fuels.insert(positive_zero));
        assert!(fuels.insert(negative_zero));
    }

    #[test]
    fn b1019_g02_brewing_fuel_legacy_marker_and_generated_defaults() {
        for marker in [NbtTag::End, NbtTag::Compound(NbtCompound::new())] {
            assert_eq!(
                BrewingFuelImpl::read_data(&marker),
                Some(BrewingFuelImpl::default())
            );
        }
        assert!(BrewingFuelImpl::read_data(&NbtTag::Int(20)).is_none());
        let mut partial = NbtCompound::new();
        partial.put_int("uses", 20);
        assert!(BrewingFuelImpl::read_data(&NbtTag::Compound(partial)).is_none());
        let mut inline = NbtCompound::new();
        inline.put("uses", NbtTag::Compound(NbtCompound::new()));
        inline.put_float("speed_multiplier", 1.0);
        assert!(BrewingFuelImpl::read_data(&NbtTag::Compound(inline)).is_none());
        let powder = crate::item_stack::ItemStack::new(1, &crate::item::Item::BLAZE_POWDER);
        assert_eq!(
            powder.get_data_component::<BrewingFuelImpl>(),
            Some(&BrewingFuelImpl::default())
        );
    }
}
