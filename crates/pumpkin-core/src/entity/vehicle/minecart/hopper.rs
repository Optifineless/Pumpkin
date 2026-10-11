use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::block::entities::hopper::HopperBlockEntity;
use crate::entity::{Entity, player::Player};
use pumpkin_data::translation;
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use super::container::{self, MinecartInventory};

#[derive(Clone)]
pub(super) struct HopperMinecart {
    enabled: Arc<AtomicBool>,
    inventory: Arc<MinecartInventory>,
    #[cfg(test)]
    container_pause: Arc<std::sync::Mutex<Option<ContainerPause>>>,
}

impl HopperMinecart {
    pub(super) fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            inventory: Arc::new(MinecartInventory::new(5)),
            #[cfg(test)]
            container_pause: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub(super) const fn inventory(&self) -> &Arc<MinecartInventory> {
        &self.inventory
    }

    pub(super) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub(super) fn tick(&self, entity: &Entity) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        let world = entity.world.load();
        let pos = entity.pos.load();
        let source_pos = BlockPos::floored(pos.x, pos.y + 1.5, pos.z);
        let inventory = &self.inventory;
        let cart_box = entity.bounding_box.load().expand(0.25, 0.0, 0.25);

        if let Some(block_entity) = world.get_block_entity(&source_pos)
            && let Some(source) = block_entity.get_inventory()
        {
            for slot in 0..source.size() {
                let stack = source.get_stack(slot);
                if stack.is_empty() || !source.can_transfer_to(inventory.as_ref(), slot, &stack) {
                    continue;
                }
                // HopperBlockEntity.tryTakeInItemFromSlot removes one item before the offer.
                let owners = pumpkin_inventory::inventory::slot_transaction::ordered_owners(&[
                    source.as_ref(),
                    inventory.as_ref(),
                ]);
                let transfers = owners
                    .iter()
                    .map(|owner| {
                        owner
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                    })
                    .collect::<Vec<_>>();
                let expected = source.get_stack(slot);
                if expected.is_empty()
                    || !source.can_transfer_to(inventory.as_ref(), slot, &expected)
                {
                    continue;
                }
                let one = pumpkin_inventory::inventory::slot_transaction::withdraw(
                    source.as_ref(),
                    slot,
                    &expected,
                    inventory.space_for_stack(&expected).min(1),
                );
                #[cfg(test)]
                self.pause_container_transfer();
                let (remaining, changed) = HopperBlockEntity::add_item_from_face_deferred(
                    Some(source.as_ref()),
                    inventory.as_ref(),
                    one.clone(),
                    None,
                );
                let moved = one.item_count.saturating_sub(remaining.item_count);
                let leftover = pumpkin_inventory::inventory::slot_transaction::refund(
                    source.as_ref(),
                    slot,
                    &expected,
                    remaining,
                );
                let source_changed =
                    !one.is_empty() && !source.get_stack(slot).are_equal(&expected);
                drop(transfers);
                if source_changed {
                    source.on_slot_changed(slot);
                }
                for slot in changed {
                    inventory.on_slot_changed(slot);
                }
                if !leftover.is_empty() {
                    world.scatter_stack(pos.x, pos.y, pos.z, leftover);
                }
                if moved > 0 {
                    return;
                }
            }
            return;
        }

        let suction_box = BoundingBox::new(
            Vector3::new(pos.x - 0.5, pos.y + 0.6875, pos.z - 0.5),
            Vector3::new(pos.x + 0.5, pos.y + 2.0, pos.z + 0.5),
        );
        if Self::pick_up_item_internal(&world, inventory, &suction_box) {
            return;
        }
        Self::pick_up_item_internal(&world, inventory, &cart_box);
    }

    fn pick_up_item_internal(
        world: &Arc<crate::world::World>,
        inventory: &Arc<MinecartInventory>,
        search_box: &BoundingBox,
    ) -> bool {
        for entity in world.get_entities_at_box(search_box) {
            let Some(item) = entity.get_item_entity() else {
                continue;
            };
            // MinecartHopper.suckInItems delegates the whole stack to HopperBlockEntity.addItem.
            if HopperBlockEntity::add_item_entity(inventory.as_ref(), item) {
                return true;
            }
        }
        false
    }

    pub(super) fn interact(
        &self,
        entity: &crate::entity::Entity,
        custom_name: Option<TextComponent>,
        player: &Arc<Player>,
    ) -> bool {
        container::open(
            entity,
            custom_name,
            player,
            &self.inventory,
            TextComponent::translate_cross(
                translation::java::ENTITY_MINECRAFT_HOPPER_MINECART,
                translation::bedrock::ENTITY_HOPPER_MINECART_NAME,
                [],
            ),
            true,
        )
    }

    pub(super) fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.inventory.write_nbt(nbt);
        nbt.put_bool("Enabled", self.enabled.load(Ordering::Relaxed));
    }

    pub(super) fn read_nbt(&self, nbt: &NbtCompound) {
        self.inventory.read_nbt(nbt);
        self.enabled
            .store(nbt.get_bool("Enabled").unwrap_or(true), Ordering::Relaxed);
    }
}

#[cfg(test)]
type ContainerPause = Box<dyn FnOnce() + Send>;

#[cfg(test)]
impl HopperMinecart {
    fn pause_container_transfer(&self) {
        let pause = self.container_pause.lock().unwrap().take();
        if let Some(pause) = pause {
            pause();
        }
    }
}

#[cfg(test)]
impl super::MinecartEntity {
    pub(crate) fn set_container_transfer_pause(&self, pause: ContainerPause) {
        let super::MinecartKind::Hopper(hopper) = &self.kind else {
            panic!("hopper required");
        };
        *hopper.container_pause.lock().unwrap() = Some(pause);
    }
}

#[cfg(test)]
mod tests {
    use super::HopperMinecart;
    use pumpkin_inventory::Inventory;

    #[test]
    fn hopper_minecart_inventory_has_five_slots() {
        assert_eq!(HopperMinecart::new().inventory.size(), 5);
    }
}
