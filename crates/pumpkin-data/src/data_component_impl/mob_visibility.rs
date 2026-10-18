use super::{DataComponentImpl, IDSet};
use crate::entity_type::EntityType;
use crate::tag::Taggable;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::identifier::Identifier;
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct MobVisibilityImpl {
    pub targeting_entity_types: IDSet<EntityType>,
    pub visibility: f32,
}

impl PartialEq for MobVisibilityImpl {
    fn eq(&self, other: &Self) -> bool {
        self.targeting_entity_types == other.targeting_entity_types
            && (self.visibility.to_bits() == other.visibility.to_bits()
                || self.visibility.is_nan() && other.visibility.is_nan())
    }
}

impl MobVisibilityImpl {
    pub const MIN_VISIBILITY: f32 = 0.0;
    pub const MAX_VISIBILITY: f32 = 10.0;

    // MobVisibility.CODEC requires both fields and bounds only the persistent float.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let visibility = compound.get_numeric_float("visibility")?;
        if visibility.total_cmp(&Self::MIN_VISIBILITY).is_lt()
            || visibility.total_cmp(&Self::MAX_VISIBILITY).is_gt()
        {
            return None;
        }
        let targets = compound.get("targeting_entity_types")?;
        let targeting_entity_types = match targets {
            NbtTag::String(name) if name.starts_with('#') => {
                let name = Identifier::parse(name.strip_prefix('#')?).ok()?.to_string();
                EntityType::get_tag_values(&name)?;
                IDSet::Tag(Cow::Owned(name))
            }
            NbtTag::String(name) => IDSet::IDs(Cow::Owned(vec![read_entity_type(name)?])),
            NbtTag::List(entries) => IDSet::IDs(Cow::Owned(
                entries
                    .iter()
                    .map(|entry| read_entity_type(entry.extract_string()?))
                    .collect::<Option<Vec<_>>>()?,
            )),
            _ => return None,
        };
        Some(Self {
            targeting_entity_types,
            visibility,
        })
    }
}

fn read_entity_type(name: &str) -> Option<&'static EntityType> {
    EntityType::from_name(&Identifier::parse(name).ok()?.to_string())
}

impl DataComponentImpl for MobVisibilityImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        let targets = match &self.targeting_entity_types {
            IDSet::Tag(tag) => NbtTag::String(format!("#{tag}").into()),
            IDSet::IDs(entities) => {
                let mut entries: Vec<_> = entities
                    .iter()
                    .map(|entity| {
                        NbtTag::String(format!("minecraft:{}", entity.resource_name).into())
                    })
                    .collect();
                // HolderSetCodec uses ExtraCodecs.compactListCodec for a single holder.
                if entries.len() == 1 {
                    entries.remove(0)
                } else {
                    NbtTag::List(entries)
                }
            }
        };
        compound.put("targeting_entity_types", targets);
        compound.put_float("visibility", self.visibility);
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> i32 {
        // MobVisibility.CODEC encoded through RegistryOps<HashOps> hashes canonical NBT values.
        super::instrument::hash::hash_tag(&self.write_data()) as i32
    }

    default_impl!(MobVisibility);
}
