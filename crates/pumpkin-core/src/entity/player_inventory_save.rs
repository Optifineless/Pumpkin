use pumpkin_data::{data_component_impl::EquipmentSlot, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory,
    inventory::slot_transaction,
    player::{ender_chest_inventory::EnderChestInventory, player_inventory::PlayerInventory},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use super::player::Player;

struct PlayerSnapshot {
    selected: u8,
    main: Vec<ItemStack>,
    equipment: Vec<(EquipmentSlot, ItemStack)>,
}

fn player_snapshot(inventory: &PlayerInventory) -> PlayerSnapshot {
    let main = inventory
        .main_inventory
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .to_vec();
    let equipment = inventory
        .entity_equipment
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .equipment
        .iter()
        .map(|(slot, stack)| (slot.clone(), stack.clone()))
        .collect();
    PlayerSnapshot {
        selected: inventory.get_selected_slot(),
        main,
        equipment,
    }
}

fn ender_snapshot(inventory: &EnderChestInventory) -> Vec<ItemStack> {
    inventory
        .items
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .to_vec()
}

// Player.addAdditionalSaveData snapshots Inventory and EnderItems at the same tick boundary.
// Pumpkin saves asynchronously, so both owners must protect the boundary, then be released
// before component serialization (which can grow to arbitrary amounts of saved data).
pub(super) fn write_both(player: &Player, nbt: &mut NbtCompound) {
    #[cfg(test)]
    pause_snapshot(player.gameprofile.id);
    let owners = slot_transaction::ordered_owners(&[
        player.inventory.as_ref() as &dyn Inventory,
        player.ender_chest_inventory.as_ref(),
    ]);
    let guards = pumpkin_inventory::inventory::slot_transaction::lock_owners(&owners);
    let main = player_snapshot(&player.inventory);
    let ender = ender_snapshot(&player.ender_chest_inventory);
    drop(guards);
    write_main(main, nbt);
    write_ender(&ender, nbt);
}

#[cfg(test)]
type SavePause = Box<dyn FnOnce() + Send>;

#[cfg(test)]
static SAVE_PAUSES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<uuid::Uuid, SavePause>>,
> = std::sync::LazyLock::new(std::sync::Mutex::default);

#[cfg(test)]
pub fn set_save_pause(player: uuid::Uuid, pause: SavePause) {
    SAVE_PAUSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(player, pause);
}

#[cfg(test)]
fn pause_snapshot(player: uuid::Uuid) {
    let pause = SAVE_PAUSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&player);
    if let Some(pause) = pause {
        pause();
    }
}

pub(super) fn write_player(inventory: &PlayerInventory, nbt: &mut NbtCompound) {
    let owner = inventory.slot_transaction(0);
    let guard = owner.lock();
    let snapshot = player_snapshot(inventory);
    drop(guard);
    write_main(snapshot, nbt);
}

pub(super) fn write_ender_chest(inventory: &EnderChestInventory, nbt: &mut NbtCompound) {
    let owner = inventory.slot_transaction(0);
    let guard = owner.lock();
    let snapshot = ender_snapshot(inventory);
    drop(guard);
    write_ender(&snapshot, nbt);
}

fn item_tag(slot: i8, stack: &ItemStack) -> NbtTag {
    let mut tag = NbtCompound::new();
    tag.put_byte("Slot", slot);
    stack.write_item_stack(&mut tag);
    NbtTag::Compound(tag)
}

fn write_main(snapshot: PlayerSnapshot, nbt: &mut NbtCompound) {
    nbt.put_int("SelectedItemSlot", i32::from(snapshot.selected));
    let mut items = Vec::new();
    for (slot, stack) in snapshot.main.iter().enumerate() {
        if !stack.is_empty() {
            items.push(item_tag(slot as i8, stack));
        }
    }
    let mut equipment = NbtCompound::new();
    for (slot, stack) in snapshot.equipment {
        if stack.is_empty() {
            continue;
        }
        let (key, index) = match slot {
            EquipmentSlot::Feet(_) => ("feet", 100),
            EquipmentSlot::Legs(_) => ("legs", 101),
            EquipmentSlot::Chest(_) => ("chest", 102),
            EquipmentSlot::Head(_) => ("head", 103),
            EquipmentSlot::OffHand(_) => ("offhand", -106),
            _ => continue,
        };
        let mut tag = NbtCompound::new();
        stack.write_item_stack(&mut tag);
        equipment.put_compound(key, tag);
        // Preserve the fork's legacy equipment entries as well as vanilla's equipment compound.
        items.push(item_tag(index, &stack));
    }
    nbt.put_compound("equipment", equipment);
    nbt.put("Inventory", NbtTag::List(items));
}

fn write_ender(snapshot: &[ItemStack], nbt: &mut NbtCompound) {
    let items = snapshot
        .iter()
        .enumerate()
        .filter(|(_, stack)| !stack.is_empty())
        .map(|(slot, stack)| item_tag(slot as i8, stack))
        .collect();
    nbt.put("EnderItems", NbtTag::List(items));
}
