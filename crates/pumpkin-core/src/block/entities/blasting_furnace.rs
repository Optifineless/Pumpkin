use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;

use std::{
    array::from_fn,
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
    },
};

use pumpkin_data::recipes::CookingRecipeKind;
use pumpkin_util::math::position::BlockPos;

use crate::{
    block::entities::furnace_like_block_entity::CookingBlockEntityBase,
    impl_block_entity_for_cooking, impl_clearable_for_cooking, impl_cooking_block_entity_base,
    impl_experience_container_for_cooking, impl_inventory_for_cooking,
    impl_property_delegate_for_cooking,
};

pub struct BlastingFurnaceBlockEntity {
    pub position: BlockPos,
    pub lock: super::container_lock::ContainerLock,
    pub dirty: AtomicBool,
    pub comparator_dirty: AtomicBool,

    pub cooking_time_spent: AtomicU16,
    pub cooking_total_time: AtomicU16,
    pub lit_time_remaining: AtomicU16,
    pub lit_total_time: AtomicU16,
    speed_multiplier: AtomicU32,

    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    pub items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,

    /// Tracks recipes used for XP calculation (vanilla `RecipesUsed` NBT format)
    /// Maps result item ID -> craft count
    pub recipes_used: StdMutex<HashMap<String, u32>>,
}

impl BlastingFurnaceBlockEntity {
    pub const INVENTORY_SIZE: usize = 3;
    pub const ID: &'static str = "minecraft:blast_furnace";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            lock: super::container_lock::ContainerLock::default(),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            cooking_total_time: AtomicU16::new(0),
            cooking_time_spent: AtomicU16::new(0),
            lit_total_time: AtomicU16::new(0),
            lit_time_remaining: AtomicU16::new(0),
            speed_multiplier: AtomicU32::new(1.0f32.to_bits()),
            recipes_used: StdMutex::new(HashMap::new()),
        }
    }
}

impl_cooking_block_entity_base!(BlastingFurnaceBlockEntity);
impl_block_entity_for_cooking!(BlastingFurnaceBlockEntity, CookingRecipeKind::Blasting);
impl_inventory_for_cooking!(BlastingFurnaceBlockEntity, CookingRecipeKind::Blasting);
impl_clearable_for_cooking!(BlastingFurnaceBlockEntity);
impl_property_delegate_for_cooking!(BlastingFurnaceBlockEntity);
impl_experience_container_for_cooking!(BlastingFurnaceBlockEntity);
