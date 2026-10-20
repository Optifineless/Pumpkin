use super::*;
use crate::data_component_impl::{PotDecoration, PotDecorationsImpl};

#[test]
fn raw_components_writer_decoded_patch_wins_over_all_raw_spellings() {
    assert_writer_collision(false);
}

#[test]
fn raw_components_writer_removed_patch_wins_over_all_raw_spellings() {
    assert_writer_collision(true);
}

#[expect(
    clippy::unwrap_used,
    reason = "Regression fixtures must serialize and reload"
)]
fn assert_writer_collision(removed: bool) {
    let mut stack = ItemStack::new(1, &Item::DIAMOND_SWORD);
    // Public native patch access can leave stale opaque copies in memory.
    stack.patch.push((
        DataComponent::Damage,
        if removed {
            None
        } else {
            Some(Box::new(DamageImpl { damage: 5 }))
        },
    ));
    for name in ["minecraft:damage", "damage", "!minecraft:damage", "!damage"] {
        stack
            .raw_components
            .push((name.into(), NbtTag::String("stale".into())));
    }
    stack
        .raw_components
        .push(("example:unrelated".into(), NbtTag::Int(37)));
    let mut saved = NbtCompound::new();
    stack.write_item_stack(&mut saved);
    let components = saved.get_compound("components").unwrap();
    assert_eq!(components.child_tags.len(), 2);
    assert_eq!(components.get_int("example:unrelated"), Some(37));
    if removed {
        assert!(
            components
                .get_compound("!minecraft:damage")
                .unwrap()
                .is_empty()
        );
        assert!(!components.has("minecraft:damage"));
    } else {
        assert_eq!(components.get_int("minecraft:damage"), Some(5));
        assert!(!components.has("!minecraft:damage"));
    }
    let restored = ItemStack::read_item_stack(&saved).unwrap();
    assert_eq!(restored.raw_components.len(), 1);
    assert_eq!(restored.has_data_component(DataComponent::Damage), !removed);
    if !removed {
        assert_eq!(restored.get_damage(), 5);
    }
}

#[test]
#[expect(
    clippy::unwrap_used,
    reason = "Regression fixtures must serialize and reload"
)]
fn raw_components_recipe_template_replaces_raw_alias_and_removal() {
    for template in [r#"{"minecraft:damage":5}"#, r#"{"!minecraft:damage":{}}"#] {
        let mut components = NbtCompound::new();
        components.put_string("damage", "future format".into());
        components.put_int("example:unrelated", 37);
        let mut saved = NbtCompound::new();
        saved.put_string("id", "minecraft:diamond_sword".into());
        saved.put_int("count", 1);
        saved.put_compound("components", components);
        let original = ItemStack::read_item_stack(&saved).unwrap();
        let result = RecipeResultStruct {
            id: "minecraft:diamond_sword",
            count: 1,
            components: Some(template),
        }
        .assemble(Some(&original), 0);
        assert_eq!(result.raw_components.len(), 1);
        assert_eq!(&*result.raw_components[0].0, "example:unrelated");
        assert_eq!(
            result.get_damage(),
            if template.contains('!') { 0 } else { 5 }
        );
        assert_eq!(
            result.has_data_component(DataComponent::Damage),
            !template.contains('!')
        );
        result.write_item_stack(&mut saved);
        assert!(result.are_equal(&ItemStack::read_item_stack(&saved).unwrap()));
        assert_eq!(original.raw_components.len(), 2);
    }
}

#[test]
#[expect(
    clippy::unwrap_used,
    reason = "Regression fixtures must serialize and reload"
)]
fn raw_components_pot_template_writer_decoded_and_removed_patch_win() {
    for removed in [false, true] {
        let face = PotDecoration {
            item: &Item::DIAMOND_SWORD,
            count: 1,
            patch: vec![(
                DataComponent::Damage,
                if removed {
                    None
                } else {
                    Some(Box::new(DamageImpl { damage: 5 }))
                },
            )],
            raw_components: ["minecraft:damage", "damage", "!minecraft:damage", "!damage"]
                .into_iter()
                .map(|name| (Box::<str>::from(name), NbtTag::String("stale".into())))
                .chain(std::iter::once((
                    Box::<str>::from("example:unrelated"),
                    NbtTag::Int(37),
                )))
                .collect(),
        };
        let pot = PotDecorationsImpl {
            decorations: [Some(face), None, None, None],
        };
        let saved = pot.write_data();
        let components = saved
            .extract_compound()
            .unwrap()
            .get_compound("back")
            .unwrap()
            .get_compound("components")
            .unwrap();
        assert_eq!(components.child_tags.len(), 2);
        assert_eq!(components.get_int("example:unrelated"), Some(37));
        let restored = PotDecorationsImpl::read_data(&saved).unwrap().decorations[0]
            .as_ref()
            .unwrap()
            .to_item_stack();
        assert_eq!(restored.raw_components.len(), 1);
        assert_eq!(restored.has_data_component(DataComponent::Damage), !removed);
        assert_eq!(restored.get_damage(), if removed { 0 } else { 5 });
    }
}
