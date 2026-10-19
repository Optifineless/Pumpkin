use std::any::Any;
use std::sync::{
    Arc, Mutex as StdMutex, RwLock,
    atomic::{AtomicBool, Ordering},
    atomic::{AtomicI32, AtomicU32},
};

use crate::block::entities::PropertyDelegate;
use pumpkin_data::data_component_impl::BrewingFuelImpl;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Potion;
use pumpkin_data::potion_brewing::BREWING_RECIPES;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::{Inventory, sync_read_items_from_nbt};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::codec::recipe::DynamicRecipe;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

#[path = "brewing_stand_state.rs"]
mod state;

pub struct BrewingStandBlockEntity {
    pub position: BlockPos,
    // recipe eligibility also includes the world's loaded recipes.
    recipe_world: StdMutex<std::sync::Weak<crate::world::World>>,
    pub(crate) transaction:
        std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction>,
    pub items: RwLock<[ItemStack; Self::INVENTORY_SIZE]>,
    pub dirty: AtomicBool,
    pub comparator_dirty: AtomicBool,
    pub brew_time: AtomicI32,
    pub fuel: AtomicI32,
    total_brew_time: AtomicI32,
    total_fuel: AtomicI32,
    speed_multiplier: AtomicU32,
    pub last_potion_count: StdMutex<Option<[bool; 3]>>,
    pub ingredient_item: StdMutex<Option<&'static pumpkin_data::item::Item>>,
}

impl BrewingStandBlockEntity {
    pub const INVENTORY_SIZE: usize = 5; // 3 potion slots + 1 ingredient + 1 fuel
    pub const ID: &'static str = "minecraft:brewing_stand";

    #[must_use]
    pub fn new(position: BlockPos) -> Self {
        use std::array::from_fn;
        Self {
            position,
            recipe_world: StdMutex::default(),
            transaction: std::sync::Arc::default(),
            items: RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
            dirty: AtomicBool::new(false),
            comparator_dirty: AtomicBool::new(false),
            brew_time: AtomicI32::new(0),
            fuel: AtomicI32::new(0),
            total_brew_time: AtomicI32::new(0),
            total_fuel: AtomicI32::new(0),
            speed_multiplier: AtomicU32::new(1.0f32.to_bits()),
            last_potion_count: StdMutex::new(None),
            ingredient_item: StdMutex::new(None),
        }
    }

    /// Vanilla `setChanged` only persists the entity, comparators react to slot changes instead.
    fn mark_timer_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Check if the current ingredient matches the stored ingredient
    fn ingredient_matches(&self, ingredient: &ItemStack) -> bool {
        self.ingredient_item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some_and(|stored| !ingredient.is_empty() && ingredient.get_item().id == stored.id)
    }

    /// Check if any potion slot has a valid recipe with the ingredient
    fn is_brewable(&self, ingredient: &ItemStack, world: &Arc<crate::world::World>) -> bool {
        if ingredient.is_empty() {
            return false;
        }

        let ingredient_id = ingredient.get_item().id;

        // Check potion recipes (water bottle -> potions, potion upgrades, etc.)
        let Ok(items) = self.items.read() else {
            return false;
        };
        for slot_idx in 0..3usize {
            let slot = &items[slot_idx];
            if slot.is_empty() {
                continue;
            }

            let slot_item_id = slot.get_item().id;
            let potion_id = slot
                .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                .and_then(|pc| pc.potion_id);

            // 1. Check data-driven BREWING_RECIPES from datapack
            for recipe in &BREWING_RECIPES {
                if slot_item_id == recipe.from_item().id
                    && ingredient_id == recipe.ingredient().id
                    && potion_id == Some(recipe.from_potion().id as i32)
                {
                    return true;
                }
            }

            // 2. Check dynamic brewing recipes from loaded datapacks
            if let Some(server) = world.server.upgrade() {
                let dynamic_recipes = server.recipe_manager.get_dynamic_recipes_internal();
                let slot_key = format!("minecraft:{}", slot.get_item().registry_key);
                let ing_key = format!("minecraft:{}", ingredient.get_item().registry_key);
                for dyn_recipe in &dynamic_recipes {
                    if let DynamicRecipe::Brewing(r) = dyn_recipe
                        && r.input_item == slot_key
                        && r.reagent == ing_key
                    {
                        if let Some(req_pot) = &r.input_potion {
                            let current_pot = potion_id
                                .and_then(|id| Potion::from_id(id as u8))
                                .map(|p| format!("minecraft:{}", p.name));
                            if current_pot.as_deref() != Some(req_pot.as_str()) {
                                continue;
                            }
                        }
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Perform the brewing process on all valid potion slots
    #[expect(clippy::too_many_lines)]
    fn do_brew(&self, world: &Arc<crate::world::World>, ingredient: &ItemStack) {
        if ingredient.is_empty() {
            return;
        }

        let ingredient_id = ingredient.get_item().id;

        // Fire BrewEvent before mutating items
        if let Some(server) = world.server.upgrade() {
            let mut brew_event = crate::plugin::api::events::inventory::brew::BrewEvent::new(
                self.position,
                self.fuel.load(Ordering::Relaxed) as u8,
            );
            server
                .plugin_manager
                .fire_blocking(&server, &mut brew_event);
            if brew_event.cancelled {
                return;
            }
        }

        // Brew potion slots
        let mut ingredient_used = false;
        let mut remainder_overflow = ItemStack::EMPTY.clone();
        {
            let owner = self.slot_transaction(0);
            let _guard = owner.lock();
            let Ok(mut items) = self.items.write() else {
                return;
            };

            if !owner.is_live() || items[3].uid != ingredient.uid || !items[3].are_equal(ingredient)
            {
                return;
            }
            for slot_idx in 0..3usize {
                let slot = &mut items[slot_idx];
                if slot.is_empty() {
                    continue;
                }

                let slot_item_id = slot.get_item().id;
                let potion_id = slot
                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                    .and_then(|pc| pc.potion_id);

                // 1. Try data-driven BREWING_RECIPES
                let mut item_brewed = false;
                for recipe in &BREWING_RECIPES {
                    if slot_item_id == recipe.from_item().id
                        && recipe.ingredient().id == ingredient_id
                        && potion_id == Some(recipe.from_potion().id as i32)
                    {
                        let mut new_slot = ItemStack::new(1, recipe.to_item());
                        let mut pc = slot
                            .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                            .cloned()
                            .unwrap_or_else(|| pumpkin_data::data_component_impl::PotionContentsImpl {
                                potion_id: None,
                                custom_color: None,
                                custom_effects: Vec::new(),
                                custom_name: None,
                            });
                        pc.potion_id = Some(recipe.to_potion().id as i32);
                        new_slot.set_data_component(pc);
                        *slot = new_slot;
                        item_brewed = true;
                        ingredient_used = true;
                        break;
                    }
                }

                if item_brewed {
                    continue;
                }

                // 2. Try dynamic brewing recipes from loaded datapacks
                if let Some(server) = world.server.upgrade() {
                    let dynamic_recipes = server.recipe_manager.get_dynamic_recipes_internal();
                    let slot_key = format!("minecraft:{}", slot.get_item().registry_key);
                    let ing_key = format!("minecraft:{}", ingredient.get_item().registry_key);
                    for dyn_recipe in &dynamic_recipes {
                        if let DynamicRecipe::Brewing(r) = dyn_recipe
                            && r.input_item == slot_key
                            && r.reagent == ing_key
                        {
                            if let Some(req_pot) = &r.input_potion {
                                let current_pot = potion_id
                                    .and_then(|id| Potion::from_id(id as u8))
                                    .map(|p| format!("minecraft:{}", p.name));
                                if current_pot.as_deref() != Some(req_pot.as_str()) {
                                    continue;
                                }
                            }
                            if let Some(target_item) = Item::from_registry_key(
                                r.output_item
                                    .strip_prefix("minecraft:")
                                    .unwrap_or(&r.output_item),
                            ) {
                                let mut new_slot = ItemStack::new(1, target_item);
                                let mut pc = slot
                                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                                    .cloned()
                                    .unwrap_or_else(|| pumpkin_data::data_component_impl::PotionContentsImpl {
                                        potion_id: None,
                                        custom_color: None,
                                        custom_effects: Vec::new(),
                                        custom_name: None,
                                    });
                                if let Some(out_pot_name) = &r.output_potion
                                    && let Some(out_pot) = Potion::from_name(
                                        out_pot_name
                                            .strip_prefix("minecraft:")
                                            .unwrap_or(out_pot_name),
                                    )
                                {
                                    pc.potion_id = Some(out_pot.id as i32);
                                }
                                new_slot.set_data_component(pc);
                                *slot = new_slot;
                                ingredient_used = true;
                                break;
                            }
                        }
                    }
                }
            }

            // Consume one ingredient
            if ingredient_used {
                remainder_overflow = Self::consume_with_remainder(&mut items[3]);
                if !self.ingredient_matches(&items[3]) {
                    *self
                        .ingredient_item
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                }
                self.brew_time.store(0, Ordering::Relaxed);
            }
        }

        if !ingredient_used {
            return;
        }

        // BrewingStandBlockEntity.serverTick finishes at zero;
        // the next tick's ordinary start path fires BrewingStartEvent.
        world.drop_stack(&self.position, remainder_overflow);

        // Play sound at the center of the block
        let pos = Vector3::new(
            self.position.0.x as f64 + 0.5,
            self.position.0.y as f64 + 0.5,
            self.position.0.z as f64 + 0.5,
        );
        world.play_sound(Sound::BlockBrewingStandBrew, SoundCategory::Blocks, &pos);

        // Mark dirty to trigger update
        self.mark_dirty();
    }
}

impl pumpkin_inventory::Inventory for BrewingStandBlockEntity {
    fn slot_transaction(
        &self,
        _slot: usize,
    ) -> std::sync::Arc<pumpkin_inventory::inventory::slot_transaction::Transaction> {
        self.transaction.clone()
    }

    fn update_slot_storage(&self, slot: usize, update: &mut dyn FnMut(&mut ItemStack)) {
        let mut items = self
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = items.get_mut(slot) else {
            return;
        };
        update(stack);
    }

    fn size(&self) -> usize {
        Self::INVENTORY_SIZE
    }

    fn is_empty(&self) -> bool {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for slot in items.iter() {
            if !slot.is_empty() {
                return false;
            }
        }
        true
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items[slot].clone()
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            removed = std::mem::replace(stack, ItemStack::EMPTY.clone());
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut removed = ItemStack::EMPTY.clone();
        self.update_slot(slot, &mut |stack| {
            if !stack.is_empty() && amount > 0 {
                removed = stack.split(amount);
            }
        });
        if !removed.is_empty() {
            self.on_slot_changed(slot);
        }
        removed
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        let mut replacement = Some(stack);
        self.update_slot(slot, &mut |current| {
            if let Some(stack) = replacement.take() {
                *current = stack;
            }
        });
        self.on_slot_changed(slot);
    }

    fn on_open(&self) {}

    fn on_close(&self) {}

    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.comparator_dirty.store(true, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // BrewingStandBlockEntity.canPlaceItem.
    fn is_valid_slot_for(&self, slot: usize, stack: &ItemStack) -> bool {
        self.can_place_item(slot, stack) && (slot >= 3 || self.get_stack(slot).is_empty())
    }

    // BrewingStandMenu.PotionSlot.mayPlace ignores occupancy; hoppers use canPlaceItem above.
    fn can_place_item(&self, slot: usize, stack: &ItemStack) -> bool {
        if stack.is_empty() {
            return false;
        }
        if slot == 4 {
            return stack.get_data_component::<BrewingFuelImpl>().is_some();
        }
        let world = self
            .recipe_world
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .upgrade();
        let dynamic = world
            .and_then(|world| world.server.upgrade())
            .map(|server| server.recipe_manager.get_dynamic_recipes_internal())
            .unwrap_or_default();
        let key = format!("minecraft:{}", stack.item.registry_key);
        match slot {
            3 => {
                // RecipePropertySet.BREWING_REAGENTS; fuel may also be a reagent.
                BREWING_RECIPES.iter().any(|recipe| recipe.ingredient() == stack.item)
                    || dynamic.iter().any(|recipe| {
                        matches!(recipe, DynamicRecipe::Brewing(recipe) if recipe.reagent == key)
                    })
            }
            0..=2 => {
                pumpkin_inventory::brewing::brewing_screen_handler::is_potion_item(stack.item)
                    || dynamic.iter().any(|recipe| {
                        matches!(recipe, DynamicRecipe::Brewing(recipe) if recipe.input_item == key)
                    })
            }
            _ => false,
        }
    }

    // BrewingStandBlockEntity.canTakeItemThroughFace (hopper below).
    fn can_transfer_to(&self, _to: &dyn Inventory, slot: usize, stack: &ItemStack) -> bool {
        slot < 3 || (slot == 3 && stack.item == &Item::GLASS_BOTTLE)
    }
}

impl pumpkin_inventory::Clearable for BrewingStandBlockEntity {
    fn clear(&self) {
        pumpkin_inventory::inventory::slot_transaction::clear(self);
    }
}

impl crate::block::entities::BlockEntity for BrewingStandBlockEntity {
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

        // Load brew time / fuel if present in NBT
        if let Some(bt) = nbt
            .get_short("BrewTime")
            .map(i32::from)
            .or_else(|| nbt.get_int("BrewTime"))
        {
            entity.brew_time.store(bt, Ordering::Relaxed);
        }
        if let Some(f) = nbt
            .get_byte("Fuel")
            .map(i32::from)
            .or_else(|| nbt.get_int("Fuel"))
        {
            entity.fuel.store(f, Ordering::Relaxed);
        }

        entity.read_brew_totals(nbt);

        // Load inventory items from NBT
        let items = entity
            .items
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sync_read_items_from_nbt(nbt, items);

        // If there's an ingredient in slot 3, remember its base item for matching
        let ingredient_item = (!items[3].is_empty()).then(|| items[3].get_item());

        // Recompute last_potion_count so visuals are correct after load
        let mut current: [bool; 3] = [false; 3];
        for (i, slot) in items.iter().take(3).enumerate() {
            current[i] = !slot.is_empty()
                && (slot
                    .get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>()
                    .is_some()
                    || slot.get_item().id == pumpkin_data::item::Item::GLASS_BOTTLE.id);
        }

        if let Some(item) = ingredient_item {
            *entity
                .ingredient_item
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(item);
        }

        *entity
            .last_potion_count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(current);

        entity
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.write_brew_inventory(nbt, true);
    }

    fn get_inventory(self: Arc<Self>) -> Option<Arc<dyn Inventory>> {
        Some(self)
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        self.write_brew_inventory(&mut nbt, false);
        Some(nbt)
    }

    fn is_comparator_dirty(&self) -> bool {
        self.comparator_dirty.load(Ordering::Relaxed)
    }

    fn clear_comparator_dirty(&self) {
        self.comparator_dirty.store(false, Ordering::Relaxed);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    #[allow(clippy::too_many_lines)]
    fn tick(&self, world: &Arc<crate::world::World>) {
        *self
            .recipe_world
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::downgrade(world);
        // Refill fuel counter from fuel item if needed
        let fuel_refilled = self.try_refill_fuel(world);

        // Get current ingredient and check brewing state
        let Ok(items) = self.items.try_read() else {
            return;
        };
        let ingredient = items[3].clone();
        drop(items);
        let brewable = self.is_brewable(&ingredient, world);
        let is_brewing = self.brew_time.load(Ordering::Relaxed) > 0;

        // Handle brewing state machine
        if is_brewing {
            if self.advance_brewing(&ingredient, brewable) {
                self.do_brew(world, &ingredient);
            }
        } else if brewable && self.fuel.load(Ordering::Relaxed) > 0 {
            if !self.start_brewing(world, &ingredient) {
                return;
            }
        } else if fuel_refilled {
            // Mark dirty if fuel was refilled to update fuel indicator
            self.mark_timer_dirty();
        }

        // Ensure clients are notified when potion slot contents (and their data) change.
        // Compute current presence bits for the three bottle slots
        let mut current: [bool; 3] = [false; 3];
        if let Ok(items_guard) = self.items.try_read() {
            for (i, slot) in items_guard.iter().take(3).enumerate() {
                // Consider a potion slot "present" when it has an item and a PotionContents component or is a glass bottle
                current[i] = !slot.is_empty() && (slot.get_data_component::<pumpkin_data::data_component_impl::PotionContentsImpl>().is_some() || slot.get_item().id == Item::GLASS_BOTTLE.id);
            }
        }

        // If potion presence changed, update last_potion_count and update block state so clients
        let mut needs_update = false;
        {
            let mut last_guard = self
                .last_potion_count
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if last_guard.as_ref() != Some(&current) {
                *last_guard = Some(current);
                needs_update = true;
            }
        }

        if needs_update {
            // Update the block state properties for the brewing stand to reflect bottle presence
            let (block, state) = world.get_block_and_state(&self.position);
            // Use generated block properties helper to produce a new state id with the bits set
            let mut props =
                pumpkin_data::block_properties::BrewingStandLikeProperties::from_state_id(state.id);
            // Generated field names use raw identifiers for clarity
            props.r#has_bottle_0 = current[0];
            props.r#has_bottle_1 = current[1];
            props.r#has_bottle_2 = current[2];

            world.set_block_state(
                &self.position,
                props.to_state_id(block),
                crate::world::BlockFlags::NOTIFY_ALL,
            );

            // Also mark dirty so inventory/container updates are sent to open screens.
            // The slot change that flipped these bits already flagged the comparator.
            self.mark_timer_dirty();
        }
    }

    fn to_property_delegate(self: Arc<Self>) -> Option<Arc<dyn PropertyDelegate>> {
        Some(self as Arc<dyn PropertyDelegate>)
    }
}

impl PropertyDelegate for BrewingStandBlockEntity {
    fn get_property(&self, index: i32) -> i32 {
        match index {
            0 => self.brew_time.load(Ordering::Relaxed),
            1 => self.fuel.load(Ordering::Relaxed),
            2 => self.total_brew_time.load(Ordering::Relaxed),
            3 => self.total_fuel.load(Ordering::Relaxed),
            _ => 0,
        }
    }

    fn set_property(&self, _index: i32, _value: i32) {}

    fn get_properties_size(&self) -> i32 {
        4
    }
}
