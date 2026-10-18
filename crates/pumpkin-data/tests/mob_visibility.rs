use pumpkin_data::{data_component::DataComponent, item_stack::ItemStack};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

fn saved_stack(targets: NbtTag, visibility: NbtTag) -> NbtCompound {
    let mut component = NbtCompound::new();
    component.put("targeting_entity_types", targets);
    component.put("visibility", visibility);
    let mut components = NbtCompound::new();
    components.put_compound("minecraft:mob_visibility", component);
    let mut stack = NbtCompound::new();
    stack.put_string("id", "minecraft:stone".into());
    stack.put_compound("components", components);
    stack
}

#[test]
fn mob_visibility_saved_stack_preserves_single_list_tag_and_removal() -> Result<(), &'static str> {
    for (targets, expected_hash) in [
        NbtTag::String("minecraft:skeleton".into()),
        NbtTag::List(vec![
            NbtTag::String("minecraft:piglin".into()),
            NbtTag::String("minecraft:piglin_brute".into()),
        ]),
        NbtTag::String("#minecraft:skeletons".into()),
    ]
    .into_iter()
    .zip([2_121_915_121, 1_309_525_365, 2_090_174_554])
    {
        let saved = saved_stack(targets.clone(), NbtTag::Double(0.25));
        let stack = ItemStack::read_item_stack(&saved).ok_or("vanilla component")?;
        assert!(stack.has_data_component(DataComponent::MobVisibility));
        // HashOps field/map/list rules checked with the 26.3 server's bundled Guava.
        let value = stack
            .patch
            .first()
            .and_then(|(_, component)| component.as_ref())
            .ok_or("component patch")?;
        assert_eq!(value.get_hash(), expected_hash);
        let mut written = NbtCompound::new();
        stack.write_item_stack(&mut written);
        let component = written
            .get_compound("components")
            .and_then(|components| components.get_compound("minecraft:mob_visibility"))
            .ok_or("retained component")?;
        assert_eq!(component.get("targeting_entity_types"), Some(&targets));
        assert_eq!(component.get_float("visibility"), Some(0.25));
        let restored = ItemStack::read_item_stack(&written).ok_or("saved component")?;
        assert!(stack.are_items_and_components_equal(&restored));
    }

    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:skeleton_skull".into());
    let mut components = NbtCompound::new();
    components.put_compound("!minecraft:mob_visibility", NbtCompound::new());
    saved.put_compound("components", components);
    let stack = ItemStack::read_item_stack(&saved).ok_or("removed default component")?;
    assert!(!stack.has_data_component(DataComponent::MobVisibility));
    let mut written = NbtCompound::new();
    stack.write_item_stack(&mut written);
    let restored = ItemStack::read_item_stack(&written).ok_or("saved removal")?;
    assert!(!restored.has_data_component(DataComponent::MobVisibility));
    Ok(())
}

#[test]
fn mob_visibility_saved_stack_rejects_invalid_components() {
    let valid_target = NbtTag::String("minecraft:skeleton".into());
    for visibility in [-0.01, -0.0, 10.01, f32::NAN, f32::INFINITY] {
        assert!(
            ItemStack::read_item_stack(&saved_stack(
                valid_target.clone(),
                NbtTag::Float(visibility)
            ))
            .is_none()
        );
    }
    for targets in [
        NbtTag::Int(1),
        NbtTag::String("minecraft:missing".into()),
        NbtTag::String("#minecraft:missing".into()),
        NbtTag::String("##minecraft:skeletons".into()),
        NbtTag::List(vec![valid_target.clone(), NbtTag::Int(1)]),
        NbtTag::List(vec![NbtTag::String("minecraft:missing".into())]),
    ] {
        assert!(ItemStack::read_item_stack(&saved_stack(targets, NbtTag::Float(0.5))).is_none());
    }
    assert!(ItemStack::read_item_stack(&saved_stack(valid_target, NbtTag::End)).is_none());
}

#[test]
fn mob_visibility_saved_stack_normalizes_default_namespace() -> Result<(), &'static str> {
    for (name, expected) in [
        ("#:skeletons", "#minecraft:skeletons"),
        (":skeleton", "minecraft:skeleton"),
    ] {
        let saved = saved_stack(NbtTag::String(name.into()), NbtTag::Float(0.5));
        let stack = ItemStack::read_item_stack(&saved).ok_or("default namespace component")?;
        let mut written = NbtCompound::new();
        stack.write_item_stack(&mut written);
        let component = written
            .get_compound("components")
            .and_then(|components| components.get_compound("minecraft:mob_visibility"))
            .ok_or("retained component")?;
        assert_eq!(
            component.get_string("targeting_entity_types"),
            Some(expected)
        );
    }
    Ok(())
}
