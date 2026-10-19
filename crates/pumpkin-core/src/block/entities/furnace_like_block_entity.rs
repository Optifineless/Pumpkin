use pumpkin_data::{item::Item, item_stack::ItemStack, recipes::CookingRecipe};

use crate::block::entities::{BlockEntity, PropertyDelegate};
pub use pumpkin_inventory::ExperienceContainer;
use pumpkin_inventory::{Clearable, Inventory};

/// Trait for extracting smelting experience from cooking block entities.
pub trait CookingBlockEntityBase:
    Sync + Send + Inventory + PropertyDelegate + BlockEntity + Clearable
{
    fn get_cooking_time_spent(&self) -> u16;
    fn get_cooking_total_time(&self) -> u16;
    fn get_lit_time_remaining(&self) -> u16;
    fn get_lit_total_time(&self) -> u16;
    fn get_speed_multiplier(&self) -> f32;

    /// Track that a recipe was used (for XP calculation on extraction)
    /// Uses the result item ID as the recipe identifier
    fn add_recipe_used(&self, recipe: &CookingRecipe);
    /// Extract and reset accumulated experience, returning the total as an integer
    /// Calculates XP from tracked recipes and clears the `recipes_used` map
    fn extract_experience_from_recipes(&self) -> Vec<i32>;
    fn get_total_cook_time(&self, recipe: &CookingRecipe, speed_multiplier: f32) -> u16;

    fn get_input_item(&self) -> ItemStack;
    fn get_fuel_item(&self) -> ItemStack;
    fn get_output_item(&self) -> ItemStack;

    fn set_cooking_time_spent(&self, spent_time: u16);
    fn set_cooking_total_time(&self, total_time: u16);
    fn set_lit_time_remaining(&self, remaining_time: u16);
    fn set_lit_total_time(&self, total_time: u16);
    fn set_speed_multiplier(&self, speed_multiplier: f32);

    fn is_burning(&self) -> bool;
    fn can_accept_recipe_output(&self, recipe: Option<&CookingRecipe>, max_count: u8) -> bool;
    fn craft_recipe(&self, recipe: Option<&CookingRecipe>) -> bool;
}

pub(crate) fn get_fuel_remainder(fuel: &ItemStack) -> Option<ItemStack> {
    // A component template carries its own count and components; create a fresh stack just as
    // AbstractFurnaceBlockEntity.consumeFuel does for an ItemStackTemplate.
    fuel.get_data_component::<pumpkin_data::data_component_impl::UseRemainderImpl>()
        .and_then(pumpkin_data::data_component_impl::UseRemainderImpl::create)
        .or_else(|| {
            // Lava buckets have a static item crafting remainder in Vanilla item data.
            (fuel.item.id == Item::LAVA_BUCKET.id).then(|| ItemStack::new(1, &Item::BUCKET))
        })
}

#[macro_export]
macro_rules! impl_cooking_block_entity_base {
    ($struct_name:ty) => {
        impl CookingBlockEntityBase for $struct_name {
            fn get_cooking_time_spent(&self) -> u16 {
                self.cooking_time_spent.load(Ordering::Relaxed)
            }

            fn get_cooking_total_time(&self) -> u16 {
                self.cooking_total_time.load(Ordering::Relaxed)
            }

            fn get_lit_time_remaining(&self) -> u16 {
                self.lit_time_remaining.load(Ordering::Relaxed)
            }

            fn get_lit_total_time(&self) -> u16 {
                self.lit_total_time.load(Ordering::Relaxed)
            }

            fn get_speed_multiplier(&self) -> f32 {
                f32::from_bits(self.speed_multiplier.load(Ordering::Relaxed))
            }

            fn get_input_item(&self) -> ItemStack {
                let items = self
                    .items
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                items[0].clone()
            }

            fn get_fuel_item(&self) -> ItemStack {
                let items = self
                    .items
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                items[1].clone()
            }

            fn get_output_item(&self) -> ItemStack {
                let items = self
                    .items
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                items[2].clone()
            }

            fn set_cooking_time_spent(&self, spent_time: u16) {
                self.cooking_time_spent.store(spent_time, Ordering::Relaxed);
            }

            fn set_cooking_total_time(&self, total_time: u16) {
                self.cooking_total_time.store(total_time, Ordering::Relaxed);
            }

            fn set_lit_time_remaining(&self, remaining_time: u16) {
                self.lit_time_remaining
                    .store(remaining_time, Ordering::Relaxed);
            }

            fn set_lit_total_time(&self, total_time: u16) {
                self.lit_total_time.store(total_time, Ordering::Relaxed);
            }

            fn set_speed_multiplier(&self, speed_multiplier: f32) {
                self.speed_multiplier
                    .store(speed_multiplier.to_bits(), Ordering::Relaxed);
            }

            fn is_burning(&self) -> bool {
                self.get_lit_time_remaining() > 0
            }

            fn add_recipe_used(&self, recipe: &pumpkin_data::recipes::CookingRecipe) {
                // Track recipe usage by recipe ID for XP calculation
                let recipe_id = recipe.recipe_id.to_string();
                let mut recipes = self
                    .recipes_used
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *recipes.entry(recipe_id).or_insert(0) += 1;
            }

            fn extract_experience_from_recipes(&self) -> Vec<i32> {
                // AbstractFurnaceBlockEntity.getRecipesToAwardAndPopExperience: one rounded
                // amount per tracked recipe, then the map is cleared.
                let mut recipes = self
                    .recipes_used
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut amounts = Vec::with_capacity(recipes.len());
                let mut random = rand::rng();
                for (recipe_id, count) in recipes.iter() {
                    // Look up the recipe's XP value
                    if let Some(xp) = pumpkin_data::recipes::get_recipe_experience(recipe_id) {
                        amounts.push(
                            $crate::block::entities::furnace_experience::recipe_experience(
                                *count,
                                xp,
                                &mut random,
                            ),
                        );
                    }
                }
                recipes.clear();
                amounts
            }

            fn get_total_cook_time(
                &self,
                recipe: &pumpkin_data::recipes::CookingRecipe,
                speed_multiplier: f32,
            ) -> u16 {
                if speed_multiplier > 0.0 {
                    (f32::from(recipe.cookingtime as u16) / speed_multiplier).ceil() as u16
                } else {
                    recipe.cookingtime as u16
                }
            }

            fn can_accept_recipe_output(
                &self,
                recipe: Option<&pumpkin_data::recipes::CookingRecipe>,
                max_count: u8,
            ) -> bool {
                let Some(recipe) = recipe else { return false };
                let Ok(items) = self.items.try_read() else {
                    return false;
                };

                let is_top_items_empty = items[0].is_empty();
                let side_item_stack = &items[2];

                if side_item_stack.is_empty() {
                    return !is_top_items_empty;
                }

                if let Some(recipe_output_item) = pumpkin_data::item::Item::from_registry_key(
                    recipe
                        .result
                        .id
                        .strip_prefix("minecraft:")
                        .unwrap_or(&recipe.result.id),
                ) && !is_top_items_empty
                    && recipe_output_item.id == side_item_stack.item.id
                    && side_item_stack.item_count < max_count
                    && side_item_stack.item_count < side_item_stack.get_max_stack_size()
                {
                    return true;
                }
                false
            }
            fn craft_recipe(&self, recipe: Option<&pumpkin_data::recipes::CookingRecipe>) -> bool {
                let owner = pumpkin_inventory::Inventory::slot_transaction(self, 0);
                let _guard = owner.lock();
                if !owner.is_live() {
                    return false;
                }
                let can_accept_output =
                    self.can_accept_recipe_output(recipe, self.get_max_count_per_stack());
                if let Some(recipe) = recipe {
                    // AbstractFurnaceBlockEntity.burn consumes only the matched ingredient.
                    if !recipe.ingredient.match_item(self.get_input_item().item) {
                        return false;
                    }
                    if can_accept_output {
                        let Ok(mut items) = self.items.try_write() else {
                            return false;
                        };
                        let Some(output_item) = pumpkin_data::item::Item::from_registry_key(
                            recipe
                                .result
                                .id
                                .strip_prefix("minecraft:")
                                .unwrap_or(&recipe.result.id),
                        ) else {
                            return false;
                        };
                        let output_item_stack = ItemStack::new(recipe.result.count, output_item);

                        if items[2].are_equal(ItemStack::EMPTY) {
                            items[2] = output_item_stack;
                        } else if items[2].are_items_and_components_equal(&output_item_stack) {
                            items[2].increment(1);
                        }

                        // Track recipe usage for XP calculation (vanilla RecipesUsed format)
                        self.add_recipe_used(recipe);

                        if items[0].item.id == pumpkin_data::item::Item::WET_SPONGE.id
                            && !items[1].is_empty()
                            && items[1].item.id == pumpkin_data::item::Item::BUCKET.id
                        {
                            items[1] = ItemStack::new(1, &pumpkin_data::item::Item::WATER_BUCKET);
                        }

                        items[0].decrement(1);
                        return true;
                    }
                }

                false
            }
        }
    };
}

#[macro_export]
macro_rules! impl_property_delegate_for_cooking {
    ($struct_name:ty) => {
        impl $crate::block::entities::PropertyDelegate for $struct_name {
            fn get_property(&self, index: i32) -> i32 {
                match index {
                    0 => self.get_lit_time_remaining() as i32,
                    1 => self.get_lit_total_time() as i32,
                    2 => self.get_cooking_time_spent() as i32,
                    3 => self.get_cooking_total_time() as i32,
                    _ => 0,
                }
            }

            fn set_property(&self, index: i32, value: i32) {
                let value = value as u16;
                match index {
                    0 => self.set_lit_time_remaining(value),
                    1 => self.set_lit_total_time(value),
                    2 => self.set_cooking_time_spent(value),
                    3 => self.set_cooking_total_time(value),
                    _ => {}
                }
            }

            fn get_properties_size(&self) -> i32 {
                4
            }
        }
    };
}

#[macro_export]
macro_rules! impl_clearable_for_cooking {
    ($struct_name:ty) => {
        impl pumpkin_inventory::Clearable for $struct_name {
            fn clear(&self) {
                pumpkin_inventory::inventory::slot_transaction::clear(self);
            }
        }
    };
}

#[macro_export]
macro_rules! impl_experience_container_for_cooking {
    ($struct_name:ty) => {
        impl $crate::block::entities::furnace_like_block_entity::ExperienceContainer
            for $struct_name
        {
            fn extract_experience_per_recipe(&self) -> Vec<i32> {
                // Delegate to the CookingBlockEntityBase method
                self.extract_experience_from_recipes()
            }
        }
    };
}

#[macro_export]
macro_rules! impl_inventory_for_cooking {
    ($struct_name:ty) => {
        $crate::impl_inventory_for_cooking!(
            $struct_name,
            pumpkin_data::recipes::CookingRecipeKind::Smelting
        );
    };
    ($struct_name:ty, $recipe_kind:expr) => {
        impl pumpkin_inventory::Inventory for $struct_name {
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

            fn on_slot_replaced(&self, slot: usize, previous: &ItemStack, current: &ItemStack) {
                self.update_cooking_input(slot, previous, current);
            }

            fn size(&self) -> usize {
                Self::INVENTORY_SIZE
            }

            fn is_empty(&self) -> bool {
                let items = self
                    .items
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                items.iter().all(|s| s.is_empty())
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
                        self.update_cooking_input(slot, current, &stack);
                        *current = stack;
                    }
                });
                self.on_slot_changed(slot);
            }

            fn mark_dirty(&self) {
                self.dirty.store(true, Ordering::Relaxed);
                self.comparator_dirty.store(true, Ordering::Relaxed);
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        impl $struct_name {
            fn update_cooking_input(&self, slot: usize, previous: &ItemStack, stack: &ItemStack) {
                // AbstractFurnaceBlockEntity.setItem resets progress when the input changes.
                let is_same_item =
                    !stack.is_empty() && previous.are_items_and_components_equal(stack);
                if slot == 0 && !is_same_item {
                    if let Some(recipe) = pumpkin_data::recipes::get_cooking_recipe_with_ingredient(
                        stack.item,
                        $recipe_kind,
                    ) {
                        self.set_cooking_total_time(
                            self.get_total_cook_time(recipe, self.get_speed_multiplier()),
                        );
                    } else {
                        self.set_cooking_total_time(0);
                    }
                    self.set_cooking_time_spent(0);
                }
            }
        }
    };
}

#[macro_export]
macro_rules! impl_block_entity_for_cooking {
    ($struct_name:ty,$recipe_kind:expr) => {
        impl $crate::block::entities::BlockEntity for $struct_name {
            fn container_lock(&self) -> Option<pumpkin_data::data_component_impl::LockImpl> { self.lock.get() }
            fn collect_item_components(&self, stack: &mut ItemStack) { self.lock.collect(stack); }
            fn apply_item_components(&self, stack: &ItemStack) { self.lock.apply(stack); }
            #[expect(clippy::too_many_lines)]
            fn tick(
                &self,
                world: &Arc<$crate::world::World>,
            ) {
                let is_burning = self.is_burning();
                let mut is_dirty = false;
                if self.is_burning() {
                    self.lit_time_remaining.fetch_sub(1, Ordering::Relaxed);
                }

                let (top_item, bottom_item) = if let Ok(items_guard) = self.items.try_read() {
                    (items_guard[0].clone(), items_guard[1].clone())
                } else {
                    return;
                };

                let is_top_items_empty = top_item.is_empty();

                let furnace_recipe = pumpkin_data::recipes::get_cooking_recipe_with_ingredient(
                    top_item.item,
                    $recipe_kind,
                );

                let can_accept_output = self
                    .can_accept_recipe_output(furnace_recipe, self.get_max_count_per_stack());

                let bottom_items_is_empty = bottom_item.is_empty();
                if self.is_burning() || !bottom_items_is_empty && !is_top_items_empty {
                    if !self.is_burning() && can_accept_output {
                        let is_fast = matches!(
                            $recipe_kind,
                            CookingRecipeKind::Blasting | CookingRecipeKind::Smoking
                        );
                        let fuel_component = bottom_item
                            .get_data_component::<pumpkin_data::data_component_impl::CookingFuelImpl>();
                        let adjusted_fuel_ticks = fuel_component
                            .map_or(0, |fuel| fuel.get_burn_ticks(is_fast));
                        let new_speed_multiplier = fuel_component
                            .map_or(1.0, |fuel| fuel.get_speed_multiplier(is_fast));

                        let mut burn_ticks = adjusted_fuel_ticks;
                        let mut burn_cancelled = false;
                        if let Some(server) = world.server.upgrade() {
                            let mut burn_event = $crate::plugin::api::events::inventory::furnace_burn::FurnaceBurnEvent::new(
                                self.position,
                                bottom_item.item.registry_key.to_string(),
                                adjusted_fuel_ticks as u32,
                            );
                            server.plugin_manager.fire_blocking(&server, &mut burn_event);
                            if burn_event.cancelled {
                                burn_cancelled = true;
                            } else {
                                burn_ticks = burn_event.burn_time as u16;
                            }
                        }
                        if !burn_cancelled {
                            let mut consumed = false;
                            let mut remainder_drop = None;
                            self.update_slot(1, &mut |current| {
                                if burn_ticks == 0 || current.uid != bottom_item.uid || !current.are_equal(&bottom_item) { return; }
                                let remainder = $crate::block::entities::furnace_like_block_entity::get_fuel_remainder(current);
                                current.decrement(1);
                                consumed = true;
                                if let Some(stack) = remainder {
                                    if current.is_empty() { *current = stack; }
                                    else { remainder_drop = Some(stack); }
                                }
                            });
                            if consumed {
                                self.set_lit_time_remaining(burn_ticks);
                                self.set_lit_total_time(burn_ticks);
                                self.set_speed_multiplier(new_speed_multiplier);
                                if let Some(recipe) = furnace_recipe {
                                    let old_total_time = self.get_cooking_total_time();
                                    let cooking_time_spent = self.get_cooking_time_spent();
                                    if old_total_time > 0 && cooking_time_spent < old_total_time {
                                        let completion_ratio =
                                            f32::from(cooking_time_spent) / f32::from(old_total_time);
                                        let new_total_time =
                                            self.get_total_cook_time(recipe, new_speed_multiplier);
                                        self.set_cooking_total_time(new_total_time);
                                        self.set_cooking_time_spent(
                                            (completion_ratio * f32::from(new_total_time)).ceil()
                                                as u16,
                                        );
                                    }
                                }
                                is_dirty = true;
                            }
                            if let Some(stack) = remainder_drop {
                                let pos = self.position.to_centered_f64();
                                let item_entity = std::sync::Arc::new($crate::entity::item::ItemEntity::new(
                                    $crate::entity::Entity::new(world.clone(), pos, &pumpkin_data::entity::EntityType::ITEM), stack,
                                ));
                                world.spawn_entity(item_entity);
                            }
                        }
                    }

                    if self.is_burning() && can_accept_output {
                        let mut start_cancelled = false;
                        if self.get_cooking_time_spent() == 0 {
                            if let Some(server) = world.server.upgrade() {
                                let mut start_event = $crate::plugin::api::events::inventory::furnace_start_smelt::FurnaceStartSmeltEvent::new(
                                    self.position,
                                    top_item.item.registry_key.to_string(),
                                    self.get_cooking_total_time() as u32,
                                );
                                server.plugin_manager.fire_blocking(&server, &mut start_event);
                                if start_event.cancelled {
                                    start_cancelled = true;
                                } else {
                                    self.set_cooking_total_time(start_event.cooking_time as u16);
                                }
                            }
                        }
                        if !start_cancelled {
                            self.cooking_time_spent.fetch_add(1, Ordering::Relaxed);

                            if self.get_cooking_time_spent() >= self.get_cooking_total_time() {
                                self.set_cooking_time_spent(0);
                                if let Some(cooking_recipe) = furnace_recipe {
                                    self.set_cooking_total_time(self.get_total_cook_time(
                                        cooking_recipe,
                                        self.get_speed_multiplier(),
                                    ));

                                    let mut smelt_cancelled = false;
                                    if let Some(server) = world.server.upgrade() {
                                        let mut smelt_event = $crate::plugin::api::events::inventory::furnace_smelt::FurnaceSmeltEvent::new(
                                            self.position,
                                            top_item.item.registry_key.to_string(),
                                            cooking_recipe.result.id.to_string(),
                                        );
                                        server.plugin_manager.fire_blocking(&server, &mut smelt_event);
                                        if smelt_event.cancelled {
                                            smelt_cancelled = true;
                                        }
                                    }
                                    if !smelt_cancelled {
                                        self.craft_recipe(Some(cooking_recipe));
                                        is_dirty = true;
                                    }
                                }
                            }
                        }
                    } else {
                        self.set_cooking_time_spent(0);
                    }
                } else if !self.is_burning() && self.get_cooking_time_spent() > 0 {
                    let _ = self.cooking_time_spent.try_update(
                        Ordering::Acquire,
                        Ordering::Acquire,
                        |v| {
                            Some(
                                v.saturating_sub(2)
                                    .min(self.cooking_total_time.load(Ordering::Acquire)),
                            )
                        },
                    );
                }

                if is_burning != self.is_burning() {
                    is_dirty = true;
                    let (furnace_block, furnace_block_state) =
                        world.get_block_and_state(&self.position);
                    let mut props =
                        pumpkin_data::block_properties::FurnaceLikeProperties::from_state_id(furnace_block_state.id);

                    props.lit = self.is_burning();
                    world.set_block_state(
                        &self.position,
                        props.to_state_id(furnace_block),
                        $crate::world::BlockFlags::NOTIFY_ALL,
                    );
                }

                if is_dirty {
                    self.mark_dirty();
                }
            }

            fn resource_location(&self) -> &'static str {
                Self::ID
            }

            fn get_position(&self) -> BlockPos {
                self.position
            }

            fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
            where
                Self: Sized,
            {
                let cooking_total_time = AtomicU16::new(
                    nbt.get_short("cooking_total_time")
                        .map_or(0, |cooking_total_time| cooking_total_time as u16),
                );
                let cooking_time_spent = AtomicU16::new(
                    nbt.get_short("cooking_time_spent")
                        .map_or(0, |cooking_time_spent| cooking_time_spent as u16),
                );
                let lit_total_time = AtomicU16::new(
                    nbt.get_short("lit_total_time")
                        .map_or(0, |lit_total_time| lit_total_time as u16),
                );
                let lit_time_remaining = AtomicU16::new(
                    nbt.get_short("lit_time_remaining")
                        .map_or(0, |lit_time_remaining| lit_time_remaining as u16),
                );
                let speed_multiplier = std::sync::atomic::AtomicU32::new(
                    nbt.get_float("cooking_speed_multiplier")
                        .unwrap_or(1.0)
                        .to_bits(),
                );
                // Load RecipesUsed from NBT (vanilla format: map of recipe ID -> craft count)
                let mut recipes_used_map = HashMap::new();
                if let Some(recipes_compound) = nbt.get_compound("RecipesUsed") {
                    for (recipe_id, tag) in &recipes_compound.child_tags {
                        if let pumpkin_nbt::tag::NbtTag::Int(count) = tag {
                            recipes_used_map.insert(recipe_id.to_string(), *count as u32);
                        }
                    }
                }

                let mut furnace = Self {
                    position,
                    lock: $crate::block::entities::container_lock::ContainerLock::from_nbt(nbt),
                    dirty: AtomicBool::new(false),
                    comparator_dirty: AtomicBool::new(false),
                    transaction: std::sync::Arc::default(),
                    items: std::sync::RwLock::new(from_fn(|_| ItemStack::EMPTY.clone())),
                    cooking_total_time,
                    cooking_time_spent,
                    lit_total_time,
                    lit_time_remaining,
                    speed_multiplier,
                    recipes_used: std::sync::Mutex::new(recipes_used_map),
                };
                pumpkin_inventory::sync_read_items_from_nbt(nbt, furnace.items.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner));

                furnace
            }

            fn write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
                self.lock.write_nbt(nbt);
                nbt.put_short("cooking_total_time", self.get_cooking_total_time() as i16);
                nbt.put_short("cooking_time_spent", self.get_cooking_time_spent() as i16);
                nbt.put_short("lit_total_time", self.get_lit_total_time() as i16);
                nbt.put_short("lit_time_remaining", self.get_lit_time_remaining() as i16);
                nbt.put_float("cooking_speed_multiplier", self.get_speed_multiplier());

                // Save RecipesUsed in vanilla format (map of recipe ID -> craft count)
                {
                    let recipes = self
                        .recipes_used
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !recipes.is_empty() {
                        let mut recipes_compound = pumpkin_nbt::compound::NbtCompound::new();
                        for (recipe_id, count) in recipes.iter() {
                            recipes_compound.put(
                                recipe_id.as_str(),
                                pumpkin_nbt::tag::NbtTag::Int(*count as i32),
                            );
                        }
                        nbt.put(
                            "RecipesUsed",
                            pumpkin_nbt::tag::NbtTag::Compound(recipes_compound),
                        );
                    }
                }

                self.write_inventory_nbt(nbt, true);
            }

            fn get_inventory(
                self: Arc<Self>,
            ) -> Option<Arc<dyn pumpkin_inventory::Inventory>> {
                Some(self)
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

            fn chunk_data_nbt(&self) -> Option<pumpkin_nbt::compound::NbtCompound> {
                let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
                nbt.put_short("cooking_total_time", self.get_cooking_total_time() as i16);
                nbt.put_short("cooking_time_spent", self.get_cooking_time_spent() as i16);
                nbt.put_short("lit_total_time", self.get_lit_total_time() as i16);
                nbt.put_short("lit_time_remaining", self.get_lit_time_remaining() as i16);
                nbt.put_float("cooking_speed_multiplier", self.get_speed_multiplier());

                if let Ok(recipes) = self.recipes_used.lock() {
                    if !recipes.is_empty() {
                        let mut recipes_compound = pumpkin_nbt::compound::NbtCompound::new();
                        for (recipe_id, count) in recipes.iter() {
                            recipes_compound.put(
                                recipe_id.as_str(),
                                pumpkin_nbt::tag::NbtTag::Int(*count as i32),
                            );
                        }
                        nbt.put(
                            "RecipesUsed",
                            pumpkin_nbt::tag::NbtTag::Compound(recipes_compound),
                        );
                    }
                }

                pumpkin_inventory::Inventory::write_inventory_nbt(self, &mut nbt, false);
                Some(nbt)
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }

            fn to_property_delegate(
                self: Arc<Self>,
            ) -> Option<Arc<dyn $crate::block::entities::PropertyDelegate>> {
                Some(self as Arc<dyn $crate::block::entities::PropertyDelegate>)
            }

            fn to_experience_container(
                self: Arc<Self>,
            ) -> Option<
                Arc<dyn $crate::block::entities::furnace_like_block_entity::ExperienceContainer>,
            > {
                Some(
                    self as Arc<
                        dyn $crate::block::entities::furnace_like_block_entity::ExperienceContainer,
                    >,
                )
            }
        }
    };
}
