use std::ops::{Deref, DerefMut};

use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use super::VillagerEntity;
use crate::entity::EntityBase;

const INVENTORY_SIZE: usize = 8;

#[derive(Clone)]
enum SavedEntry {
    Slot(usize),
    Raw(NbtTag),
}

/// The eight usable villager slots and unrepresentable saved entries.
#[derive(Clone)]
pub struct VillagerInventory {
    slots: [ItemStack; INVENTORY_SIZE],
    // Markers keep raw entries interleaved with the successfully decoded stacks.
    saved_entries: Vec<SavedEntry>,
    warned: bool,
}

impl Default for VillagerInventory {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| ItemStack::EMPTY.clone()),
            saved_entries: Vec::new(),
            warned: false,
        }
    }
}

impl Deref for VillagerInventory {
    type Target = [ItemStack; INVENTORY_SIZE];

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl DerefMut for VillagerInventory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}

impl VillagerInventory {
    // InventoryCarrier.readInventoryFromTag / SimpleContainer.fromItemList (26.3).
    // Keep stack order and retain rejected/overflow entries for lossless migration.
    fn from_item_list(list: &[NbtTag]) -> Self {
        let mut inventory = Self::default();
        let mut next_slot = 0;
        for tag in list {
            let compound = tag.extract_compound();
            // The old fork wrote canonical AIR stacks for unused slots. Do not
            // treat arbitrary zero-count records as empty: they may be unknown data.
            if compound.is_some_and(Self::is_legacy_empty_air) {
                continue;
            }
            let stack = compound.and_then(Self::read_representable_stack);
            if let Some(stack) = stack
                && next_slot < INVENTORY_SIZE
            {
                inventory.slots[next_slot] = stack;
                inventory.saved_entries.push(SavedEntry::Slot(next_slot));
                next_slot += 1;
            } else {
                inventory.saved_entries.push(SavedEntry::Raw(tag.clone()));
            }
        }
        inventory
    }

    fn is_legacy_empty_air(entry: &NbtCompound) -> bool {
        entry.get_string("id") == Some("minecraft:air")
            && entry.get_int("count") == Some(0)
            && entry.child_tags.iter().all(|(name, tag)| match name.as_ref() {
                "id" | "count" => true,
                "components" => matches!(tag, NbtTag::Compound(components) if components.child_tags.is_empty()),
                _ => false,
            })
    }

    fn read_representable_stack(entry: &NbtCompound) -> Option<ItemStack> {
        if entry
            .child_tags
            .keys()
            .any(|name| !matches!(name.as_ref(), "id" | "count" | "components"))
        {
            return None;
        }
        let components = match entry.get("components") {
            Some(NbtTag::Compound(components)) => Some(components),
            Some(_) => return None,
            None => None,
        };
        let stack = ItemStack::read_item_stack(entry)?;
        if stack.is_empty() {
            return None;
        }

        // A successful decode is not enough: component codecs can normalize or
        // skip payloads. Keep the source entry raw unless its component patch
        // comes back unchanged from the writer.
        let mut encoded = NbtCompound::new();
        stack.write_item_stack(&mut encoded);
        let encoded_components = encoded.get_compound("components");
        if let Some(components) = components {
            if encoded_components != Some(components) {
                return None;
            }
        } else if encoded_components.is_some_and(|components| !components.child_tags.is_empty()) {
            return None;
        }
        Some(stack)
    }

    // InventoryCarrier.writeInventoryToTag / SimpleContainer.storeAsItemList (26.3).
    // Called on an owned snapshot after releasing the inventory guard.
    pub(super) fn write_nbt(&self, nbt: &mut NbtCompound) {
        let mut list = Vec::new();
        let mut saved_slots = [false; INVENTORY_SIZE];
        for entry in &self.saved_entries {
            match entry {
                SavedEntry::Slot(index) => {
                    saved_slots[*index] = true;
                    Self::write_stack(&self.slots[*index], &mut list);
                }
                SavedEntry::Raw(tag) => list.push(tag.clone()),
            }
        }
        for (index, stack) in self.slots.iter().enumerate() {
            if !saved_slots[index] {
                Self::write_stack(stack, &mut list);
            }
        }
        nbt.put_list("Inventory", list);
    }

    fn write_stack(stack: &ItemStack, list: &mut Vec<NbtTag>) {
        if !stack.is_empty() {
            let mut compound = NbtCompound::new();
            stack.write_item_stack(&mut compound);
            list.push(NbtTag::Compound(compound));
        }
    }
}

impl VillagerEntity {
    pub(super) fn read_inventory_nbt(&self, nbt: &NbtCompound) {
        let Some(list) = nbt.get_list("Inventory") else {
            return;
        };
        let mut loaded = VillagerInventory::from_item_list(list);
        let needs_warning = loaded
            .saved_entries
            .iter()
            .any(|entry| matches!(entry, SavedEntry::Raw(_)));
        let warn = {
            let mut inventory = self
                .inventory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let warn = needs_warning && !inventory.warned;
            loaded.warned = inventory.warned || needs_warning;
            *inventory = loaded;
            warn
        };
        if warn {
            tracing::warn!(
                villager = %self.get_entity().entity_uuid,
                "Villager inventory contains unsupported or excess entries; preserving saved data"
            );
        }
    }
}
