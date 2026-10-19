use super::BrewingStandBlockEntity;
use pumpkin_data::{data_component_impl::BrewingFuelImpl, item_stack::ItemStack};
use pumpkin_inventory::{Inventory, crafting::crafting_remainder};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{Arc, atomic::Ordering};

impl BrewingStandBlockEntity {
    pub(super) fn advance_brewing(&self, ingredient: &ItemStack, brewable: bool) -> bool {
        let owner = self.slot_transaction(0);
        let _guard = owner.lock();
        let current = self.get_stack(3);
        let remaining = self.brew_time.load(Ordering::Relaxed);
        if !owner.is_live()
            || remaining <= 0
            || current.uid != ingredient.uid
            || !current.are_equal(ingredient)
        {
            return false;
        }
        let remaining = remaining - 1;
        self.brew_time.store(remaining, Ordering::Relaxed);
        self.mark_timer_dirty();
        if remaining == 0 && brewable {
            return true;
        }
        if !brewable || !self.ingredient_matches(ingredient) {
            self.brew_time.store(0, Ordering::Relaxed);
        }
        false
    }

    // BrewingStandBlockEntity.doBrew/serverTick use Item.getCraftingRemainder.
    pub(super) fn consume_with_remainder(stack: &mut ItemStack) -> ItemStack {
        let remainder = crafting_remainder(stack.item);
        stack.decrement(1);
        if stack.is_empty() {
            *stack = remainder;
            ItemStack::EMPTY.clone()
        } else {
            remainder
        }
    }

    pub(super) fn try_refill_fuel(&self, world: &Arc<crate::world::World>) -> bool {
        let expected_fuel = if self.fuel.load(Ordering::Relaxed) <= 0 {
            self.get_stack(4)
        } else {
            return false;
        };
        let Some(component) = expected_fuel.get_data_component::<BrewingFuelImpl>() else {
            return false;
        };
        if expected_fuel.is_empty() {
            return false;
        }
        let Some((uses, speed)) = crate::command::commands::evaluate_block_providers(
            world,
            self.position,
            &component.uses.to_nbt(),
            &component.speed_multiplier.to_nbt(),
        ) else {
            return false;
        };

        let mut fuel_power = uses;
        if let Some(server) = world.server.upgrade() {
            // Keep the native event's u16 payload without narrowing the provider's saved counter.
            let event_power = u16::try_from(uses).unwrap_or(if uses < 0 { 0 } else { u16::MAX });
            let mut event = crate::plugin::api::events::inventory::brewing_stand_fuel::BrewingStandFuelEvent::new(
                self.position, event_power,
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
            if event.fuel_power != event_power {
                fuel_power = i32::from(event.fuel_power);
            }
        }

        let mut refilled = false;
        let mut overflow = ItemStack::EMPTY.clone();
        self.update_slot(4, &mut |current| {
            if self.fuel.load(Ordering::Relaxed) <= 0
                && current.uid == expected_fuel.uid
                && current.are_equal(&expected_fuel)
                && !current.is_empty()
            {
                overflow = Self::consume_with_remainder(current);
                self.fuel.store(fuel_power, Ordering::Relaxed);
                self.total_fuel.store(fuel_power, Ordering::Relaxed);
                self.speed_multiplier
                    .store(speed.to_bits(), Ordering::Relaxed);
                refilled = true;
            }
        });
        if refilled {
            self.on_slot_changed(4);
            world.drop_stack(&self.position, overflow);
        }
        refilled
    }

    pub(super) fn start_brewing(
        &self,
        world: &Arc<crate::world::World>,
        ingredient: &ItemStack,
    ) -> bool {
        let (expected, fuel, speed) = {
            let owner = self.slot_transaction(0);
            let _guard = owner.lock();
            if !owner.is_live() {
                return false;
            }
            let items = self
                .items
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if items[3].uid != ingredient.uid || !items[3].are_equal(ingredient) {
                return false;
            }
            (
                items.clone(),
                self.fuel.load(Ordering::Relaxed),
                self.speed_multiplier.load(Ordering::Relaxed),
            )
        };
        if fuel <= 0 || self.brew_time.load(Ordering::Relaxed) > 0 {
            return false;
        }
        let speed_multiplier = f32::from_bits(speed);
        let speed_multiplier = if speed_multiplier > 0.0 {
            speed_multiplier
        } else {
            1.0
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Vanilla ceil(400.0F / speedMultiplier)"
        )]
        let mut brew_time = (400.0f32 / speed_multiplier).ceil() as i32;
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::block::brewing_start::BrewingStartEvent::new(
                    self.position,
                    world.clone(),
                    brew_time,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
            brew_time = event.brewing_time;
        }

        let owner = self.slot_transaction(0);
        let _guard = owner.lock();
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !owner.is_live()
            || self.brew_time.load(Ordering::Relaxed) > 0
            || self.fuel.load(Ordering::Relaxed) != fuel
            || self.speed_multiplier.load(Ordering::Relaxed) != speed
            || items
                .iter()
                .zip(&expected)
                .any(|(current, old)| current.uid != old.uid || !current.are_equal(old))
        {
            return false;
        }
        self.fuel.store(fuel - 1, Ordering::Relaxed);
        self.brew_time.store(brew_time, Ordering::Relaxed);
        self.total_brew_time.store(brew_time, Ordering::Relaxed);
        *self
            .ingredient_item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ingredient.item);
        self.mark_timer_dirty();
        true
    }

    pub(super) fn read_brew_totals(&self, nbt: &NbtCompound) {
        self.total_brew_time.store(
            nbt.get_int("total_brew_time").unwrap_or(400),
            Ordering::Relaxed,
        );
        self.total_fuel
            .store(nbt.get_int("total_fuel").unwrap_or(20), Ordering::Relaxed);
        self.speed_multiplier.store(
            nbt.get_float("speed_multiplier").unwrap_or(1.0).to_bits(),
            Ordering::Relaxed,
        );
    }

    pub(super) fn write_brew_inventory(&self, nbt: &mut NbtCompound, include_empty: bool) {
        let owner = self.slot_transaction(0);
        let guard = owner.lock();
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        nbt.put_int("BrewTime", self.brew_time.load(Ordering::Relaxed));
        nbt.put_int("Fuel", self.fuel.load(Ordering::Relaxed));
        nbt.put_int(
            "total_brew_time",
            self.total_brew_time.load(Ordering::Relaxed),
        );
        nbt.put_int("total_fuel", self.total_fuel.load(Ordering::Relaxed));
        nbt.put_float(
            "speed_multiplier",
            f32::from_bits(self.speed_multiplier.load(Ordering::Relaxed)),
        );
        drop(guard);
        if include_empty {
            nbt.put_list("Items", Vec::new());
        }
        pumpkin_inventory::sync_write_items_to_nbt(&items, nbt);
    }
}
