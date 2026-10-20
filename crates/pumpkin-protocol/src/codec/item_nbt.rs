use pumpkin_data::{data_component::DataComponent, item_stack::ItemStack};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

/// Projects saved items inside an owned outgoing entity or block-entity NBT copy.
#[must_use]
pub fn project_item_nbt_for_client(mut nbt: NbtCompound) -> NbtCompound {
    project_compound(&mut nbt);
    nbt
}

fn project_compound(nbt: &mut NbtCompound) {
    if nbt.has("components")
        && let Some(stack) = ItemStack::read_item_stack(nbt)
        && let Some(NbtTag::Compound(components)) = nbt.child_tags.get_mut("components")
    {
        for (name, _) in stack.raw_components {
            if name
                .strip_prefix('!')
                .and_then(DataComponent::try_from_name)
                .is_some()
            {
                // Keep the decoded removal while withholding its opaque payload.
                components.put_compound(&name, NbtCompound::new());
            } else {
                components.child_tags.remove(&name);
            }
        }
        for (name, tag) in &mut components.child_tags {
            if DataComponent::try_from_name(name.strip_prefix('!').unwrap_or(name))
                != Some(DataComponent::CustomData)
            {
                project_tag(tag);
            }
        }
        // A block entity can also have an item ID and a components compound.
        for (name, tag) in &mut nbt.child_tags {
            if name.as_ref() != "components" {
                project_tag(tag);
            }
        }
    } else {
        for tag in nbt.child_tags.values_mut() {
            project_tag(tag);
        }
    }
}

fn project_tag(tag: &mut NbtTag) {
    match tag {
        NbtTag::Compound(nbt) => project_compound(nbt),
        NbtTag::List(tags) => {
            for tag in tags {
                project_tag(tag);
            }
        }
        _ => {}
    }
}
