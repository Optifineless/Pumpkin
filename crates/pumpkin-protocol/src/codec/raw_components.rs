use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BeesImpl, BlockEntityDataImpl, BucketEntityDataImpl, DataComponentImpl, EntityDataImpl,
    },
    item_stack::ItemStack,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

pub(super) fn can_serialize_component(id: DataComponent, data: &dyn DataComponentImpl) -> bool {
    // These components carry entity/block-entity inventories as generic NBT rather than
    // typed item templates. Keep an unsafe parent server-side; CustomData stays arbitrary.
    let data = data.as_any();
    match id {
        DataComponent::BlockEntityData => data
            .downcast_ref::<BlockEntityDataImpl>()
            .is_none_or(|data| !compound_has_raw_stack(&data.nbt)),
        DataComponent::EntityData => data
            .downcast_ref::<EntityDataImpl>()
            .and_then(|data| data.nbt.as_ref())
            .is_none_or(|nbt| !compound_has_raw_stack(nbt)),
        DataComponent::BucketEntityData => data
            .downcast_ref::<BucketEntityDataImpl>()
            .and_then(|data| data.nbt.as_ref())
            .is_none_or(|nbt| !compound_has_raw_stack(nbt)),
        DataComponent::Bees => data.downcast_ref::<BeesImpl>().is_none_or(|data| {
            data.bees
                .iter()
                .all(|bee| !compound_has_raw_stack(&bee.entity_data))
        }),
        _ => true,
    }
}

fn compound_has_raw_stack(nbt: &NbtCompound) -> bool {
    if nbt.has("components")
        && ItemStack::read_item_stack(nbt).is_some_and(|stack| !stack.raw_components.is_empty())
    {
        return true;
    }
    nbt.child_tags.values().any(tag_has_raw_stack)
}

fn tag_has_raw_stack(tag: &NbtTag) -> bool {
    match tag {
        NbtTag::Compound(nbt) => compound_has_raw_stack(nbt),
        NbtTag::List(tags) => tags.iter().any(tag_has_raw_stack),
        _ => false,
    }
}
