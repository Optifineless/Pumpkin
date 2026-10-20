//! Unknown save fields belong to the live entity, including across partial NBT writes.

use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::{NbtCompound, read_tracking::NbtRead};

use super::Entity;

pub(super) fn read_item_stack(nbt: &NbtCompound) -> Option<ItemStack> {
    // Component patches are one owned payload; removed components must not return.
    let _ = nbt.get("components");
    ItemStack::read_item_stack(nbt)
}

pub(super) fn read(entity: &Entity, nbt: &NbtCompound, read: impl FnOnce()) {
    let reads = NbtRead::capture(|| {
        // EntityType.loadPassengersRecursive owns these outside the ordinary reader.
        // They must not survive dismounting, nor override a newly constructed identity.
        let _ = nbt.get("id");
        let _ = nbt.get("UUID");
        let _ = nbt.get("Passengers");
        read();
    });
    entity
        .retained_nbt
        .rcu(|previous| reads.retain(nbt, previous));
}

impl Entity {
    pub(crate) fn retain_only_present_nbt(&self, nbt: &NbtCompound) {
        self.retained_nbt.rcu(|previous| {
            let mut retained = (**previous).clone();
            retained.retain_present(nbt);
            retained
        });
    }

    pub(super) fn write_retained_nbt(&self, nbt: &mut NbtCompound) {
        self.retained_nbt.load_full().write_to(nbt);
    }
}
