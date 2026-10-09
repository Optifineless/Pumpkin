use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::generic_container_screen_handler::{create_generic_9x3, create_hopper};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_inventory::{Clearable, Inventory};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use crate::entity::{Entity, player::Player};

/// Shared chest-vehicle inventory with deferred loot and a single destruction claim.
pub struct MinecartInventory {
    items: RwLock<Vec<ItemStack>>,
    size: usize,
    loot_table: Mutex<Option<(String, i64)>>,
    drops_claimed: AtomicBool,
}

impl MinecartInventory {
    pub(crate) fn new(size: usize) -> Self {
        Self {
            items: RwLock::new(vec![ItemStack::EMPTY.clone(); size]),
            size,
            loot_table: Mutex::new(None),
            drops_claimed: AtomicBool::new(false),
        }
    }

    pub(crate) fn claim_drops(&self) -> bool {
        !self.drops_claimed.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn read_nbt(&self, nbt: &NbtCompound) {
        let loot_table = nbt.get_string("LootTable").map(|loot_table| {
            (
                loot_table.to_owned(),
                nbt.get_long("LootTableSeed").unwrap_or(0),
            )
        });
        let has_loot_table = loot_table.is_some();
        if let Ok(mut guard) = self.loot_table.try_lock() {
            *guard = loot_table;
        }

        if !has_loot_table && let Ok(mut items) = self.items.try_write() {
            items.fill_with(|| ItemStack::EMPTY.clone());
            self.read_data(nbt, &mut items);
        }
    }

    pub(crate) fn write_nbt(&self, nbt: &mut NbtCompound) {
        // ContainerEntity.addChestVehicleSaveData always saves deferred loot or Items.
        // Guards stay inside inventory methods and are released before serialization.
        let loot_table = self
            .loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some((loot_table, seed)) = loot_table {
            nbt.put_string("LootTable", loot_table);
            if seed != 0 {
                nbt.put_long("LootTableSeed", seed);
            }
        } else {
            let items = self
                .items
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let mut list: Vec<pumpkin_nbt::tag::NbtTag> = Vec::new();
            for (slot, stack) in items.iter().enumerate() {
                if !stack.is_empty() {
                    let mut compound = NbtCompound::new();
                    compound.put_byte("Slot", slot as i8);
                    stack.write_item_stack(&mut compound);
                    list.push(pumpkin_nbt::tag::NbtTag::Compound(compound));
                }
            }
            nbt.put("Items", pumpkin_nbt::tag::NbtTag::List(list));
        }
    }

    pub(super) fn has_loot_table(&self) -> bool {
        self.loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    pub(crate) fn unpack_loot(
        self: &Arc<Self>,
        params: &crate::world::loot::LootContextParameters,
    ) {
        let loot_table = self
            .loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some((loot_table, seed)) = loot_table else {
            return;
        };
        let Some(table) = params.world.as_ref().map_or_else(
            || crate::world::loot::get_loot_table(&loot_table),
            |world| world.get_loot_table(&loot_table),
        ) else {
            *self
                .loot_table
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((loot_table, seed));
            return;
        };

        let inventory: Arc<dyn Inventory> = self.clone();
        crate::world::loot::fill_chest_inventory_with_context(&inventory, &table, seed, params);
    }
}

impl Inventory for MinecartInventory {
    fn size(&self) -> usize {
        self.size
    }

    fn is_empty(&self) -> bool {
        let items = self
            .items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.iter().all(ItemStack::is_empty)
    }

    fn get_stack(&self, slot: usize) -> ItemStack {
        self.items
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[slot]
            .clone()
    }

    fn remove_stack(&self, slot: usize) -> ItemStack {
        let mut items = self
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut items[slot], ItemStack::EMPTY.clone())
    }

    fn remove_stack_specific(&self, slot: usize, amount: u8) -> ItemStack {
        let mut items = self
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !items[slot].is_empty() && amount > 0 {
            items[slot].split(amount)
        } else {
            ItemStack::EMPTY.clone()
        }
    }

    fn set_stack(&self, slot: usize, stack: ItemStack) {
        self.items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[slot] = stack;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Clearable for MinecartInventory {
    fn clear(&self) {
        self.items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fill_with(|| ItemStack::EMPTY.clone());
    }
}

struct MinecartScreenFactory {
    inventory: Arc<MinecartInventory>,
    title: TextComponent,
    hopper: bool,
    vehicle: std::sync::Weak<dyn crate::entity::EntityBase>,
    player: std::sync::Weak<Player>,
}

impl ScreenHandlerFactory for MinecartScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let inventory: Arc<dyn Inventory> = self.inventory.clone();
        let mut handler = if self.hopper {
            create_hopper(sync_id, player_inventory, inventory, player)
        } else {
            create_generic_9x3(sync_id, player_inventory, inventory, player)
        };
        let vehicle = self.vehicle.clone();
        let player = self.player.clone();
        handler.validity_check = Some(Box::new(move || {
            // AbstractChestBoat.stillValid -> ContainerEntity.isChestVehicleStillValid.
            vehicle
                .upgrade()
                .zip(player.upgrade())
                .is_some_and(|(vehicle, player)| {
                    let entity = vehicle.get_entity();
                    let range = player.living_entity.get_attribute_value(
                        &pumpkin_data::attributes::Attributes::ENTITY_INTERACTION_RANGE,
                    ) + 4.0;
                    !entity.is_removed()
                        && Arc::ptr_eq(&entity.world.load_full(), &player.world())
                        && entity
                            .bounding_box
                            .load()
                            .squared_magnitude(player.eye_position())
                            < range * range
                })
        }));
        Some(Arc::new(Mutex::new(handler)) as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        self.title.clone()
    }
}

/// Opens a vehicle inventory after materializing its deferred loot for this player.
pub fn open(
    entity: &Entity,
    custom_name: Option<TextComponent>,
    player: &Arc<Player>,
    inventory: &Arc<MinecartInventory>,
    title: TextComponent,
    hopper: bool,
) -> bool {
    if player.is_spectator() && inventory.has_loot_table() {
        return false;
    }
    if !player.is_spectator() {
        let params = crate::world::loot::build_container_loot_context(
            &entity.world.load_full(),
            entity.pos.load(),
            Some(player),
        );
        inventory.unpack_loot(&params);
    }

    let Some(vehicle) = entity.world.load().get_entity_by_id(entity.entity_id) else {
        return false;
    };
    player
        .open_handled_screen(
            &MinecartScreenFactory {
                inventory: inventory.clone(),
                title: custom_name.unwrap_or(title),
                hopper,
                vehicle: Arc::downgrade(&vehicle),
                player: Arc::downgrade(player),
            },
            None,
        )
        .is_some()
}

pub(super) fn velocity(
    entity: &Entity,
    inventory: &MinecartInventory,
    velocity: Vector3<f64>,
) -> Vector3<f64> {
    let has_loot = inventory
        .loot_table
        .try_lock()
        .is_ok_and(|guard| guard.is_some());
    let signal = if has_loot {
        0
    } else if let Ok(items) = inventory.items.try_read() {
        let mut total_fill = 0.0;
        let mut has_items = false;
        for stack in items.iter() {
            if !stack.is_empty() {
                let max_count = stack.get_max_stack_size();
                total_fill += f64::from(stack.item_count) / f64::from(max_count);
                has_items = true;
            }
        }
        if has_items {
            let factor = total_fill / inventory.size as f64;
            (factor * 14.0).floor() as u8 + 1
        } else {
            0
        }
    } else {
        0
    };
    let mut friction = if has_loot {
        0.98
    } else {
        0.98 + f64::from(15 - signal) * 0.001
    };
    if entity
        .touching_water
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        friction *= 0.95;
    }
    velocity.multiply(friction, 0.0, friction)
}

#[cfg(test)]
mod tests {
    use super::MinecartInventory;
    use pumpkin_data::item::Item;
    use pumpkin_data::item_stack::ItemStack;
    use pumpkin_inventory::Inventory;
    use pumpkin_nbt::compound::NbtCompound;

    fn save_while_locked<T>(
        inventory: &MinecartInventory,
        guard: T,
    ) -> Result<NbtCompound, Box<dyn std::error::Error>> {
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::Duration;

        std::thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (saved_tx, saved_rx) = mpsc::channel();
            scope.spawn(move || {
                assert!(started_tx.send(()).is_ok());
                let mut nbt = NbtCompound::new();
                inventory.write_nbt(&mut nbt);
                assert!(saved_tx.send(nbt).is_ok());
            });
            let started = started_rx.recv_timeout(Duration::from_secs(5));
            let early_save = saved_rx.recv_timeout(Duration::from_millis(100));
            // Release before asserting, so a failure cannot strand the scoped saver.
            drop(guard);
            started?;
            assert!(
                matches!(early_save, Err(RecvTimeoutError::Timeout)),
                "saving must wait for the held inventory lock"
            );
            Ok(saved_rx.recv_timeout(Duration::from_secs(5))?)
        })
    }

    #[test]
    fn regression_save_waits_for_contended_inventory_lock() -> Result<(), Box<dyn std::error::Error>>
    {
        let inventory = MinecartInventory::new(27);
        inventory.set_stack(8, ItemStack::new(3, &Item::POWERED_RAIL));
        let guard = inventory
            .items
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let nbt = save_while_locked(&inventory, guard)?;

        assert!(nbt.get_list("Items").is_some());
        let restored = MinecartInventory::new(27);
        restored.read_nbt(&nbt);
        assert_eq!(restored.get_stack(8).item, &Item::POWERED_RAIL);
        assert_eq!(restored.get_stack(8).item_count, 3);
        Ok(())
    }

    #[test]
    fn regression_save_waits_for_contended_loot_table_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        let inventory = MinecartInventory::new(27);
        let mut source = NbtCompound::new();
        source.put_string(
            "LootTable",
            "minecraft:chests/abandoned_mineshaft".to_owned(),
        );
        source.put_long("LootTableSeed", 1234);
        inventory.read_nbt(&source);
        let guard = inventory
            .loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let nbt = save_while_locked(&inventory, guard)?;

        assert_eq!(nbt.get_string("LootTable"), source.get_string("LootTable"));
        assert_eq!(nbt.get_long("LootTableSeed"), Some(1234));
        assert!(nbt.get_list("Items").is_none());
        Ok(())
    }

    #[test]
    fn deferred_mineshaft_loot_is_preserved_until_unpacked() {
        let inventory = std::sync::Arc::new(MinecartInventory::new(27));
        let mut source = NbtCompound::new();
        source.put_string(
            "LootTable",
            "minecraft:chests/abandoned_mineshaft".to_string(),
        );
        source.put_long("LootTableSeed", 1234);
        inventory.read_nbt(&source);

        let mut deferred = NbtCompound::new();
        inventory.write_nbt(&mut deferred);
        assert_eq!(
            deferred.get_string("LootTable"),
            Some("minecraft:chests/abandoned_mineshaft")
        );
        assert_eq!(deferred.get_long("LootTableSeed"), Some(1234));
        assert!(deferred.get_list("Items").is_none());

        inventory.unpack_loot(&crate::world::loot::LootContextParameters::default());
        assert!(!inventory.is_empty());

        let mut unpacked = NbtCompound::new();
        inventory.write_nbt(&mut unpacked);
        assert!(unpacked.get_string("LootTable").is_none());
        assert!(unpacked.get_list("Items").is_some());
    }

    #[test]
    fn chest_minecart_items_round_trip_through_nbt() {
        let inventory = MinecartInventory::new(27);
        inventory.set_stack(8, ItemStack::new(3, &Item::POWERED_RAIL));

        let mut nbt = NbtCompound::new();
        inventory.write_nbt(&mut nbt);

        let restored = MinecartInventory::new(27);
        restored.read_nbt(&nbt);
        let stack = restored.get_stack(8);
        assert_eq!(stack.get_item().id, Item::POWERED_RAIL.id);
        assert_eq!(stack.item_count, 3);
    }
}
