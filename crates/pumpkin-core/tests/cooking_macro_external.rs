use std::{
    array::from_fn,
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
    },
};

use pumpkin_core::{
    block::entities::{
        BlockEntity, container_lock::ContainerLock,
        furnace_like_block_entity::CookingBlockEntityBase,
    },
    impl_block_entity_for_cooking, impl_clearable_for_cooking, impl_cooking_block_entity_base,
    impl_experience_container_for_cooking, impl_inventory_for_cooking,
    impl_property_delegate_for_cooking,
};
use pumpkin_data::{item::Item, item_stack::ItemStack, recipes::CookingRecipeKind};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;

// This integration target expands the public macros across a real crate boundary,
// as a native plugin does. Built-in furnace expansions cannot detect private helpers.
struct PluginCookingEntity {
    position: BlockPos,
    lock: ContainerLock,
    dirty: AtomicBool,
    comparator_dirty: AtomicBool,
    cooking_time_spent: AtomicI32,
    cooking_total_time: AtomicI32,
    lit_time_remaining: AtomicI32,
    lit_total_time: AtomicI32,
    speed_multiplier: AtomicU32,
    transaction: Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    recipes_used: Mutex<HashMap<String, u32>>,
}

impl PluginCookingEntity {
    const INVENTORY_SIZE: usize = 3;
    const ID: &'static str = "test:plugin_cooking";
}

impl_cooking_block_entity_base!(PluginCookingEntity);
impl_block_entity_for_cooking!(PluginCookingEntity, CookingRecipeKind::Smelting);
impl_inventory_for_cooking!(PluginCookingEntity, CookingRecipeKind::Smelting);
impl_clearable_for_cooking!(PluginCookingEntity);
impl_property_delegate_for_cooking!(PluginCookingEntity);
impl_experience_container_for_cooking!(PluginCookingEntity);

#[test]
fn exported_cooking_macros_support_native_plugin_entities() {
    let entity = PluginCookingEntity::from_nbt(&NbtCompound::new(), BlockPos::new(0, 64, 0));
    entity.set_stack(0, ItemStack::new(2, &Item::RAW_IRON));
    entity.set_speed_multiplier(2.0);
    let mut saved = NbtCompound::new();
    entity.write_internal(&mut saved);
    let restored = PluginCookingEntity::from_nbt(&saved, entity.get_position());
    assert_eq!(restored.get_speed_multiplier(), 2.0);
    assert!(
        restored
            .get_stack(0)
            .are_equal(&ItemStack::new(2, &Item::RAW_IRON))
    );
}
