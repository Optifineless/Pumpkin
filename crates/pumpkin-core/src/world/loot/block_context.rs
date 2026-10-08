use super::LootContextParameters;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomNameImpl, DataComponentImpl, read_data},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::{collections::HashMap, sync::Arc};

/// Collect typed block components once, before loot modifiers and plugin events.
#[must_use]
pub fn collect_block_entity_components(
    world: &Arc<crate::world::World>,
    pos: &BlockPos,
) -> HashMap<DataComponent, Box<dyn DataComponentImpl>> {
    // BlockEntity.collectComponents/collectImplicitComponents: never serialize typed contents.
    let Some(entity) = world.get_block_entity(pos) else {
        return HashMap::new();
    };
    let mut stack = ItemStack::new(1, &Item::AIR);
    entity.collect_item_components(&mut stack);
    let mut result: HashMap<_, _> = stack
        .patch
        .into_iter()
        .filter_map(|(id, value)| value.map(|value| (id, value)))
        .collect();
    // Banners have no typed collection hook yet. Decode their native NBT directly.
    // Bees and pot decorations have placeholder codecs and remain unsupported.
    if entity.resource_location() == "minecraft:banner" {
        let mut nbt = NbtCompound::new();
        entity.write_nbt(&mut nbt);
        if let Some(patterns) = nbt.get("patterns")
            && let Some(component) = read_data(DataComponent::BannerPatterns, patterns)
        {
            result.insert(DataComponent::BannerPatterns, component);
        }
        if let Some(name) = nbt.get_string("CustomName")
            && let Ok(name) = serde_json::from_str(name)
        {
            result.insert(DataComponent::CustomName, Box::new(CustomNameImpl { name }));
        }
    }
    if let Some(pot) = entity
        .as_any()
        .downcast_ref::<crate::block::entities::decorated_pot::DecoratedPotBlockEntity>()
        && let Some(item) = pot
            .item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
    {
        result.insert(
            DataComponent::Container,
            Box::new(pumpkin_data::data_component_impl::ContainerImpl {
                items: vec![(0, item.clone())],
            }),
        );
    }
    result
}

/// Attach the live block context and collect components before evaluating drops.
#[must_use]
pub fn build_block_loot_context(
    world: &Arc<crate::world::World>,
    pos: &BlockPos,
    base: &LootContextParameters,
) -> LootContextParameters {
    LootContextParameters {
        world: Some(world.clone()),
        registry: world
            .server
            .upgrade()
            .map(|server| server.datapack_manager.clone()),
        block_entity_components: collect_block_entity_components(world, pos),
        ..base.clone()
    }
}
