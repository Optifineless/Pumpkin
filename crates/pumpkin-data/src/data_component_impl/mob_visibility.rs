use super::{DataComponentImpl, IDSet};
use crate::entity::EntityType;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[derive(Clone, Debug, PartialEq)]
pub struct MobVisibilityImpl {
    pub targeting_entity_types: IDSet<EntityType>,
    pub visibility: f32,
}

impl MobVisibilityImpl {
    // MobVisibility.CODEC: a registry holder set and a float in [0, 10].
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let compound = data.extract_compound()?;
        let encoded_types = compound.get("targeting_entity_types")?;
        let targeting_entity_types = IDSet::read(encoded_types)?;
        if let (NbtTag::List(values), IDSet::IDs(types)) = (encoded_types, &targeting_entity_types)
            && values.len() != types.len()
        {
            return None;
        }
        let visibility = super::instrument::number(compound.get("visibility")?)?;
        (0.0..=10.0).contains(&visibility).then_some(Self {
            targeting_entity_types,
            visibility,
        })
    }
}

impl DataComponentImpl for MobVisibilityImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        // HolderSetCodec's compactListCodec writes a singleton as a namespaced registry name.
        let types = match &self.targeting_entity_types {
            IDSet::IDs(types) => {
                let mut names: Vec<_> = types
                    .iter()
                    .map(|entity_type| {
                        NbtTag::String(format!("minecraft:{}", entity_type.resource_name).into())
                    })
                    .collect();
                if names.len() == 1 {
                    names.remove(0)
                } else {
                    NbtTag::List(names)
                }
            }
            IDSet::Tag(tag) => NbtTag::String(
                if tag.contains(':') {
                    format!("#{tag}")
                } else {
                    format!("#minecraft:{tag}")
                }
                .into(),
            ),
        };
        compound.put("targeting_entity_types", types);
        compound.put_float("visibility", self.visibility);
        NbtTag::Compound(compound)
    }

    fn get_hash(&self) -> i32 {
        // MobVisibility.CODEC hashes the record with HashOps, including its field names.
        super::instrument::hash::hash_tag(&self.write_data()) as i32
    }
    default_impl!(MobVisibility);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data_component::DataComponent, data_component_impl::read_data};

    #[test]
    fn avoidance_followup_mob_visibility_hash_matches_vanilla() {
        // MobVisibility.CODEC.encodeStart(RegistryOps(HashOps.CRC32C_INSTANCE), value).asInt(), 26.3.
        let singleton = MobVisibilityImpl {
            targeting_entity_types: IDSet::IDs(vec![&EntityType::SKELETON].into()),
            visibility: 0.5,
        };
        assert_eq!(singleton.get_hash(), 8_201_492);
        let two = MobVisibilityImpl {
            targeting_entity_types: IDSet::IDs(
                vec![&EntityType::PIGLIN, &EntityType::PIGLIN_BRUTE].into(),
            ),
            visibility: 0.5,
        };
        assert_eq!(two.get_hash(), 805_911_184);
    }

    #[test]
    fn avoidance_followup_mob_visibility_codec_preserves_heads_and_rejects_invalid_visibility() {
        let mut compound = NbtCompound::new();
        compound.put_string("targeting_entity_types", "minecraft:skeleton".to_owned());
        compound.put_float("visibility", 0.5);
        let encoded = NbtTag::Compound(compound.clone());
        let component = read_data(DataComponent::MobVisibility, &encoded).unwrap();
        let decoded = component
            .as_any()
            .downcast_ref::<MobVisibilityImpl>()
            .unwrap();
        assert_eq!(decoded.visibility, 0.5);
        assert!(
            matches!(&decoded.targeting_entity_types, IDSet::IDs(types) if types.as_ref() == [&EntityType::SKELETON])
        );
        assert_eq!(component.write_data(), encoded);
        for visibility in [-0.1, 10.1, f32::NAN] {
            compound.put_float("visibility", visibility);
            assert!(
                read_data(
                    DataComponent::MobVisibility,
                    &NbtTag::Compound(compound.clone())
                )
                .is_none()
            );
        }
        compound.put_float("visibility", 0.5);
        compound.put_list(
            "targeting_entity_types",
            vec![NbtTag::String("minecraft:missing".into())],
        );
        assert!(read_data(DataComponent::MobVisibility, &NbtTag::Compound(compound)).is_none());
    }
}
