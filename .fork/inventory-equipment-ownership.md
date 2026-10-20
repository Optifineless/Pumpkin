# Inventory and equipment ownership for mob visibility

Reviewed against `17239d945daa12da488fd163e9231fa643355c18` before
changing the visibility read path for fork PR #2. This records existing ownership;
it does not certify the legacy inventory surface as migrated.

| State | Authoritative storage and existing access | Writers and derived readers |
|:--|:--|:--|
| Selected hotbar slot | `PlayerInventory.selected_slot`, validated `set_selected_slot` / `get_selected_slot`; atomic, separate from stack ownership | Java `set_held_item` / `pick_item`; Bedrock `mob_equipment`, block pick and use-item selection reconciliation; inventory hotbar picking; NBT load; native/Wasm inventory calls |
| Player main hand | `PlayerInventory.main_inventory[selected_slot]`; `held_item` clones under the existing main-inventory read lock | `Inventory::update_slot` / slot transactions, hand exchanges, menu clicks, crafting, item use, pickups, give/clear, death/respawn and NBT load; equipment modifiers, attacks, publication and visibility must read the selected stack |
| Player armor and offhand | Shared `LivingEntity.entity_equipment` / `PlayerInventory.entity_equipment`; existing equipment lock; inventory indices map through `equipment_slots` | Inventory slot transactions, equipment swaps/damage, armor-stand interactions, commands, item use, death/respawn, NBT and plugin writes |
| Non-player equipment | `LivingEntity.entity_equipment`, including main hand | Mob finalization/load, equipment changes/damage/drop and plugin writers; equipment modifiers, enchantments, publication and visibility read this storage |
| Published player main hand | Equipment packets and equipment/modifier publication queues are derived state; an equipment-map MAIN_HAND entry is not authoritative | `player/equipment_updates.rs`, `living/equipment_modifiers.rs`, `living/equipment_packets.rs`; slot/selection changes can precede publication |

`pumpkin-inventory/src/player/player_inventory.rs` owns the existing slot
transaction plus main-inventory/equipment storage. `inventory/slot_transaction.rs`
claims physical containers for conditional edits. Existing operations needing
both storage locks use main inventory before equipment. The new visibility reader
reuses `LivingEntity::snapshot_equipped_stacks`, which reads the player's selected
hand before snapshotting equipment, replaces any cached MAIN_HAND entry and returns
owned stacks. Component inspection and target selection run after snapshot guards
have ended. It adds no owner, lock, inventory mutation or packet/event dispatch.
Snapshots of separate slot families are not a new atomic inventory transaction.

Villager biological age (slice 6) is a separate level-30 checked owner in
`entity/passive/villager/age.rs`. It owns `Age`, `ForcedAge`, `AgeLocked`, the
age-only schema provenance and retained undecoded tags. Writers are hydration,
the biological setter (including egg/cure configuration) and one species tick;
semantic baby readers, geometry and metadata derive from its scalar snapshot.
The age writer captures an owned image and releases the guard before encoding;
normal save and mark-before-capture chunk retirement use that same owner.
Removed writes reject under the owner guard. Exact tracker/base, world and
entity-chunk storage observations bound unlocked effects. This does not make
inventory, food, HOME and age a combined birth transaction. External lifecycle,
ticks and staged-enforcement briefs are not available in this checkout.

Important callers and transitions reviewed:

- Java and Bedrock selection handlers dispatch the held-item event before updating
  selection. Cancelled changes keep the old slot. Visibility reads the admitted
  current selection, including before the equipment publisher catches up.
- `/give`, `/clear`, menu/hand edits and durability changes modify live stack
  storage. Visibility has no cache to invalidate and does not depend on equipment
  packet delivery or modifier refresh.
- `entity/player_inventory_save.rs` captures selected slot, main storage and
  armor/offhand using the existing inventory owner, then serializes owned stacks.
  `PlayerInventory::read_nbt` restores `SelectedItemSlot` and `Inventory`;
  living equipment loading retains the existing equipment format. Vanilla and
  earlier fork saves without `mob_visibility` keep item defaults. Explicit
  components use the normal item component read/write path.
- Disconnect/rejoin and server restart use those saved values. Death/respawn use
  `entity/player/death.rs` and existing keep-inventory rules. Dimension transfers
  retain the current player's inventory. No visibility snapshot survives these
  operations or chunk/entity unload.
- Native and Wasm inventory/equipment entry points retain their current contracts;
  no public plugin API or event payload changes. Existing mutable public storage
  and independently atomic selection remain legacy ownership limitations.

The hotbar regression must drive actual target-goal selection with a custom
component loaded through item NBT, change selection through the carried-item
handler and leave a stale MAIN_HAND equipment entry. This distinguishes live
inventory authority from its derived publication state without adding pause hooks.
