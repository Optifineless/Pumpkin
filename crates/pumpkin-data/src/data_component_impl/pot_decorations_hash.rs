use super::{PotDecoration, PotDecorationsImpl};
use crate::data_component_impl::{Digest, get_i32_hash, get_str_hash};

// PotDecorations.CODEC and ItemStackTemplate.CODEC encoded through HashOps.CRC32C_INSTANCE.
pub(super) fn get_hash(decorations: &PotDecorationsImpl) -> i32 {
    hash_map(
        decorations
            .decorations
            .iter()
            .zip(PotDecorationsImpl::FACES)
            .filter_map(|(face, name)| {
                face.as_ref()
                    .map(|face| (get_str_hash(name), template_hash(face)))
            })
            .collect(),
    ) as i32
}

fn template_hash(template: &PotDecoration) -> u32 {
    let mut fields = vec![(
        get_str_hash("id"),
        get_str_hash(&format!("minecraft:{}", template.item.registry_key)),
    )];
    if template.count != 1 {
        fields.push((get_str_hash("count"), get_i32_hash(template.count)));
    }
    // OptionalFieldCodec tests the original patch against EMPTY before its codec
    // filters transient components, so a transient-only patch still emits {}.
    if !template.patch.is_empty() {
        let components = template
            .patch
            .iter()
            .filter(|(id, _)| PotDecoration::is_persistent_component(*id))
            .map(|(id, value)| match value {
                Some(value) => (get_str_hash(id.to_name()), value.get_hash() as u32),
                None => (
                    get_str_hash(&format!("!{}", id.to_name())),
                    hash_map(Vec::new()),
                ),
            })
            .collect();
        fields.push((get_str_hash("components"), hash_map(components)));
    }
    hash_map(fields)
}

fn hash_map(mut fields: Vec<(u32, u32)>) -> u32 {
    // HashOps compares HashCode.padToLong, then feeds HashCode.asBytes to CRC32C.
    fields.sort_unstable();
    let mut digest = Digest::new();
    digest.update(&[2]);
    for (key, value) in fields {
        digest.update(&key.to_le_bytes());
        digest.update(&value.to_le_bytes());
    }
    digest.update(&[3]);
    digest.finalize() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data_component::DataComponent,
        data_component_impl::{DamageImpl, DataComponentImpl, MapPostProcessingImpl},
        item::Item,
    };

    fn brick(count: i32) -> PotDecoration {
        PotDecoration {
            item: &Item::BRICK,
            count,
            patch: Vec::new(),
        }
    }

    #[test]
    fn b1020b_g09_pot_hash_matches_independently_encoded_vanilla_maps() {
        // Fixed CRC32C fixtures independently encoded from vanilla 26.3 HashOps,
        // PotDecorations.CODEC, ItemStackTemplate.CODEC and DataComponentPatch.CODEC.
        assert_eq!(PotDecorationsImpl::EMPTY.get_hash() as u32, 0xc574_b4c8);
        let mut pot = PotDecorationsImpl::EMPTY;
        pot.decorations[0] = Some(brick(1));
        assert_eq!(pot.get_hash() as u32, 0xdcb2_3b6b);
        pot.decorations.swap(0, 1);
        assert_eq!(pot.get_hash() as u32, 0x9e78_54b7);
        pot.decorations.swap(0, 1);
        pot.decorations[0] = Some(brick(2));
        assert_eq!(pot.get_hash() as u32, 0x50da_5060);
        pot.decorations[2] = Some(PotDecoration {
            item: &Item::ANGLER_POTTERY_SHERD,
            ..brick(1)
        });
        assert_eq!(pot.get_hash() as u32, 0xf803_a480);

        let mut face = brick(2);
        face.patch = vec![
            (
                DataComponent::Damage,
                Some(DamageImpl { damage: 3 }.to_dyn()),
            ),
            (DataComponent::Unbreakable, None),
        ];
        pot.decorations = [Some(face.clone()), None, None, None];
        assert_eq!(pot.get_hash() as u32, 0x0a92_6050);
        face.patch.reverse();
        pot.decorations[0] = Some(face);
        assert_eq!(pot.get_hash() as u32, 0x0a92_6050);
    }

    #[test]
    fn b1020b_g09_transient_patch_entries_keep_the_components_field() {
        let mut face = brick(1);
        face.patch = vec![(
            DataComponent::MapPostProcessing,
            Some(MapPostProcessingImpl::LOCK.to_dyn()),
        )];
        let pot = PotDecorationsImpl {
            decorations: [Some(face), None, None, None],
        };
        assert_eq!(pot.get_hash() as u32, 0x89c7_eab8);
    }
}
