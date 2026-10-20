use super::BlockEntity;
use crate::entity::Entity;
use crate::entity::item::ItemEntity;
use crate::world::World;
use pumpkin_data::Block;
use pumpkin_data::block_properties::CampfireLikeProperties;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::{CookingRecipeKind, get_cooking_recipe_with_ingredient};
use pumpkin_inventory::{Clearable, Inventory, inventory::slot_transaction};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::chunk::io::Dirtiable;
use rand::{RngExt, rng};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

mod food;
#[cfg(test)]
mod tests;

pub struct CampfireBlockEntity {
    pub position: BlockPos,
    pub items: [Arc<Mutex<ItemStack>>; Self::SLOT_COUNT],
    pub cooking_times: [Mutex<i32>; Self::SLOT_COUNT],
    pub cooking_total_times: [Mutex<i32>; Self::SLOT_COUNT],
    cooking_started: [AtomicBool; Self::SLOT_COUNT],
    transaction: Arc<slot_transaction::Transaction>,
    dirty: AtomicBool,
}

impl BlockEntity for CampfireBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let mut entity = Self::new(position);

        if let Some(list) = nbt.get_list("Items") {
            for tag in list {
                if let Some(compound) = tag.extract_compound() {
                    let slot = compound.get_byte("Slot").unwrap_or(0) as usize;
                    if slot < Self::SLOT_COUNT
                        && let Some(stack) = ItemStack::read_item_stack(compound)
                    {
                        entity.items[slot] = Arc::new(Mutex::new(stack));
                    }
                }
            }
        }

        if let Some(arr) = nbt.get_int_array("CookingTimes") {
            for (slot, &value) in arr.iter().enumerate().take(Self::SLOT_COUNT) {
                entity.cooking_times[slot] = Mutex::new(value);
            }
        }
        if let Some(arr) = nbt.get_int_array("CookingTotalTimes") {
            for (slot, &value) in arr.iter().enumerate().take(Self::SLOT_COUNT) {
                entity.cooking_total_times[slot] = Mutex::new(value);
            }
        }
        // Old fork and vanilla saves have no marker and retain the legacy <=0 start path.
        if let Some(arr) = nbt.get_byte_array("CookingStarted") {
            for (slot, &value) in arr.iter().enumerate().take(Self::SLOT_COUNT) {
                entity.cooking_started[slot].store(value != 0, Ordering::Relaxed);
            }
        }

        entity
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_cooking_nbt(nbt);
    }

    fn tick(&self, world: &Arc<World>) {
        let block = world.get_block(&self.position);
        if block != &Block::CAMPFIRE && block != &Block::SOUL_CAMPFIRE {
            return;
        }

        let state_id = world.get_block_state(&self.position).id;
        let properties = CampfireLikeProperties::from_state_id(state_id);

        if !properties.lit {
            if self.cool_down() {
                self.mark_chunk_dirty(world);
            }
            return;
        }

        let mut changed = false;
        let mut completed = false;
        for slot in 0..Self::SLOT_COUNT {
            let (advanced, cooked) = self.cook_slot(world, slot);
            changed |= advanced;
            completed |= cooked;
        }

        if completed {
            if let Some(block_entity) = world.get_block_entity(&self.position) {
                world.update_block_entity(&block_entity);
            }
        } else if changed {
            self.mark_chunk_dirty(world);
        }
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_cooking_nbt(&mut nbt);
        Some(nbt)
    }

    fn on_block_replaced(self: Arc<Self>, world: &Arc<World>, _position: &BlockPos) {
        // CampfireBlockEntity.preRemoveSideEffects drains settled food before scattering.
        for stack in slot_transaction::retire_and_drain(self.as_ref()) {
            world.scatter_stack(
                f64::from(self.position.0.x),
                f64::from(self.position.0.y),
                f64::from(self.position.0.z),
                stack,
            );
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl CampfireBlockEntity {
    pub const ID: &'static str = "minecraft:campfire";
    pub const SLOT_COUNT: usize = 4;

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        Self {
            position,
            items: std::array::from_fn(|_| Arc::new(Mutex::new(ItemStack::EMPTY.clone()))),
            cooking_times: std::array::from_fn(|_| Mutex::new(0)),
            cooking_total_times: std::array::from_fn(|_| Mutex::new(0)),
            cooking_started: std::array::from_fn(|_| AtomicBool::new(false)),
            transaction: Arc::default(),
            dirty: AtomicBool::new(false),
        }
    }

    fn write_cooking_nbt(&self, nbt: &mut NbtCompound) {
        let guard = self.transaction.lock();
        let snapshot = (0..Self::SLOT_COUNT)
            .map(|slot| self.get_stack(slot))
            .collect::<Vec<_>>();
        let cooking_times = self
            .cooking_times
            .iter()
            .map(|time| {
                *time
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect();
        let cooking_total_times = self
            .cooking_total_times
            .iter()
            .map(|time| {
                *time
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect();
        let cooking_started = self
            .cooking_started
            .iter()
            .map(|started| i8::from(started.load(Ordering::Relaxed)))
            .collect();
        drop(guard);
        let mut items = Vec::new();
        for (slot, stack) in snapshot.into_iter().enumerate() {
            if !stack.is_empty() {
                let mut item_nbt = NbtCompound::new();
                item_nbt.put_byte("Slot", slot as i8);
                stack.write_item_stack(&mut item_nbt);
                items.push(NbtTag::Compound(item_nbt));
            }
        }
        nbt.put_list("Items", items);
        nbt.put("CookingTimes", NbtTag::IntArray(cooking_times));
        nbt.put("CookingTotalTimes", NbtTag::IntArray(cooking_total_times));
        nbt.put("CookingStarted", NbtTag::ByteArray(cooking_started));
    }

    fn cool_down(&self) -> bool {
        let _guard = self.transaction.lock();
        if !self.transaction.is_live() {
            return false;
        }
        let mut changed = false;
        // Mirrors vanilla 26.3 CampfireBlockEntity.cooldownTick.
        for slot in 0..Self::SLOT_COUNT {
            let mut progress = self.cooking_times[slot]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *progress > 0 {
                let total = *self.cooking_total_times[slot]
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let cooled = (*progress - 2).max(0).min(total);
                if cooled != *progress {
                    *progress = cooled;
                    changed = true;
                }
            }
        }
        changed
    }

    // CampfireBlockEntity.cookTick admits consumption after the existing cook event.
    fn cook_slot(&self, world: &Arc<World>, slot: usize) -> (bool, bool) {
        let stack = slot_transaction::snapshot_slot(self, slot);
        if stack.is_empty() {
            return (false, false);
        }
        let Some(recipe) =
            get_cooking_recipe_with_ingredient(stack.item, CookingRecipeKind::CampfireCooking)
        else {
            return (false, false);
        };
        let mut total = *self.cooking_total_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if total <= 0 && !self.cooking_started[slot].load(Ordering::Relaxed) {
            total = recipe.cookingtime;
            if let Some(server) = world.server.upgrade() {
                let mut event =
                    crate::plugin::api::events::block::campfire_start::CampfireStartEvent::new(
                        self.position,
                        world.clone(),
                        stack.clone(),
                        slot as u8,
                        total,
                    );
                server.plugin_manager.fire_blocking(&server, &mut event);
                if event.cancelled {
                    return (false, false);
                }
                total = event.cooking_time;
            }
        }
        let guard = self.transaction.lock();
        let current = self.get_stack(slot);
        if !self.transaction.is_live() || current.uid != stack.uid || !current.are_equal(&stack) {
            return (false, false);
        }
        let mut saved_total = self.cooking_total_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *saved_total <= 0 && !self.cooking_started[slot].load(Ordering::Relaxed) {
            *saved_total = total;
            self.cooking_started[slot].store(true, Ordering::Relaxed);
        }
        total = *saved_total;
        drop(saved_total);
        let mut progress = self.cooking_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *progress += 1;
        let finished = *progress >= total;
        drop(progress);
        drop(guard);
        if !finished {
            return (true, false);
        }
        let Some(result_item) = Item::from_registry_key(recipe.result.id) else {
            return (true, false);
        };
        let mut result = ItemStack::new(recipe.result.count, result_item);
        if let Some(server) = world.server.upgrade() {
            let mut event = crate::plugin::api::events::block::block_cook::BlockCookEvent::new(
                self.position,
                world.clone(),
                stack.clone(),
                result.clone(),
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return (true, false);
            }
            result = event.result;
        }
        let guard = self.transaction.lock();
        let consumed = slot_transaction::withdraw(self, slot, &stack, stack.item_count);
        if consumed.is_empty() {
            return (true, false);
        }
        *self.cooking_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0;
        *self.cooking_total_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0;
        self.cooking_started[slot].store(false, Ordering::Relaxed);
        drop(guard);
        Self::spawn_cooked_item(world, self.position, result);
        (true, true)
    }

    fn mark_chunk_dirty(&self, world: &Arc<World>) {
        let chunk_position = self.position.chunk_position();
        let _ = world.level.read_chunk_sync(&chunk_position, |chunk| {
            chunk.mark_dirty(true);
        });
    }

    fn spawn_cooked_item(world: &Arc<World>, position: BlockPos, stack: ItemStack) {
        let spawn_position = Vector3::new(
            f64::from(position.0.x) + 0.5 + rng().random_range(-0.35..0.35),
            f64::from(position.0.y) + 1.01,
            f64::from(position.0.z) + 0.5 + rng().random_range(-0.35..0.35),
        );
        let entity = Entity::new(world.clone(), spawn_position, &EntityType::ITEM);
        world.spawn_entity(Arc::new(ItemEntity::new(entity, stack)));
    }
}

// Private cooking slots use the same ownership contract without exposing a hopper container.
impl Inventory for CampfireBlockEntity {
    fn slot_transaction(&self, _slot: usize) -> Arc<slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        if let Some(item) = self.items.get(slot) {
            update(
                &mut item
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        }
    }

    fn on_slot_replaced(&self, slot: usize, _previous: &ItemStack, current: &ItemStack) {
        self.cooking_started[slot].store(false, Ordering::Relaxed);
        // CampfireBlockEntity.placeFood initializes the timer with the admitted food.
        *self.cooking_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0;
        *self.cooking_total_times[slot]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            get_cooking_recipe_with_ingredient(current.item, CookingRecipeKind::CampfireCooking)
                .map_or(0, |recipe| recipe.cookingtime);
    }

    fn size(&self) -> usize {
        Self::SLOT_COUNT
    }

    fn is_empty(&self) -> bool {
        (0..Self::SLOT_COUNT).all(|slot| self.get_stack(slot).is_empty())
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        self.items.get(slot).map_or_else(
            || ItemStack::EMPTY.clone(),
            |item| {
                item.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            },
        )
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut taken = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            taken = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });
        if !taken.is_empty() {
            self.on_slot_changed(slot);
        }
        taken
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut taken = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            taken = stack.split(amount.min(stack.item_count));
        });
        if !taken.is_empty() {
            self.on_slot_changed(slot);
        }
        taken
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                self.on_slot_replaced(slot, current, &stack);
                *current = stack;
            }
        });
        self.on_slot_changed(slot);
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Clearable for CampfireBlockEntity {
    fn clear(&self) {
        slot_transaction::clear(self);
    }
}
