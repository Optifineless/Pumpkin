//! `ProjectileDispenseBehavior` admission from the original, still-owned dispenser slot.

use super::DispenseContext;
use crate::{block::entities::dispenser::DispenserBlockEntity, entity::EntityBase};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use std::sync::Arc;

pub(super) fn is_projectile(item: &Item) -> bool {
    [
        Item::ARROW.id,
        Item::TIPPED_ARROW.id,
        Item::SPECTRAL_ARROW.id,
        Item::SNOWBALL.id,
        Item::EGG.id,
        Item::SPLASH_POTION.id,
        Item::LINGERING_POTION.id,
        Item::FIRE_CHARGE.id,
        Item::WIND_CHARGE.id,
        Item::FIREWORK_ROCKET.id,
        Item::EXPERIENCE_BOTTLE.id,
    ]
    .contains(&item.id)
}

pub(super) fn launch(ctx: &DispenseContext<'_>, projectile: Arc<dyn EntityBase>) -> bool {
    let Some((source, slot, expected)) = ctx.projectile_source else {
        return false;
    };
    let Some(prepared) =
        crate::entity::projectile::launch::prepare_projectile(ctx.world, projectile, None)
    else {
        return false;
    };
    // Resolve pending/replaced containers before acquiring the inventory owner. Removal and
    // unloading retire that owner, so neither an obsolete source nor a recreated one can pay.
    let Some(current) = ctx.world.get_block_entity(ctx.position) else {
        return false;
    };
    if !current
        .as_any()
        .downcast_ref::<DispenserBlockEntity>()
        .is_some_and(|dispenser| std::ptr::eq(dispenser, source))
    {
        return false;
    }
    let mut replacement = expected.clone();
    replacement.decrement(1);
    if replacement.is_empty() {
        replacement = ItemStack::EMPTY.clone();
    }
    let inserted = ctx.world.insert_admitted_projectile(prepared, |append| {
        let owner = source.slot_transaction(slot);
        let _guard = owner.lock();
        let actual = source.get_stack(slot);
        if !owner.is_live()
            || expected.is_empty()
            || actual.uid != expected.uid
            || !actual.are_equal(expected)
        {
            return false;
        }
        if !append() {
            return false;
        }
        source.update_slot_storage(slot, &mut |stack| {
            let previous = std::mem::replace(stack, replacement.clone());
            source.on_slot_replaced(slot, &previous, stack);
        });
        true
    });
    if inserted {
        source.on_slot_changed(slot);
    }
    inserted
}
