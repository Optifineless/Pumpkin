use crate::{container_click::MouseClick, screen_handler::InventoryPlayer, slot::Slot};
use pumpkin_data::{data_component_impl::BundleContentsImpl, item_stack::ItemStack, sound::Sound};
use std::sync::Arc;

pub fn override_click(
    slot: &Arc<dyn Slot>,
    cursor: &mut ItemStack,
    click: &MouseClick,
    player: &dyn InventoryPlayer,
) -> bool {
    // BundleItem.overrideStackedOnOther is tried before overrideOtherStackedOnMe.
    let other = slot.get_stack();
    if let Some(contents) = cursor.get_data_component_mut::<BundleContentsImpl>() {
        if *click == MouseClick::Left && !other.is_empty() {
            let max = contents.max_amount_to_add(&other);
            let mut taken = slot.safe_take(other.item_count, max, player);
            let inserted = contents.try_insert(&mut taken);
            player.play_bundle_sound(if inserted {
                Sound::ItemBundleInsert
            } else {
                Sound::ItemBundleInsertFail
            });
            return true;
        }
        if *click == MouseClick::Right && other.is_empty() {
            if let Some(extracted) = contents.try_extract() {
                let mut remainder = slot.insert_stack(extracted);
                if remainder.is_empty() {
                    player.play_bundle_sound(Sound::ItemBundleRemoveOne);
                } else {
                    contents.try_insert(&mut remainder);
                }
            }
            return true;
        }
    }
    let mut other = other;
    if let Some(contents) = other.get_data_component_mut::<BundleContentsImpl>() {
        if *click == MouseClick::Left && cursor.is_empty() {
            contents.selected_item = -1;
            slot.set_stack(other);
            return false;
        }
        if *click == MouseClick::Left && !cursor.is_empty() {
            let inserted = slot.allow_modification(player) && contents.try_insert(cursor);
            player.play_bundle_sound(if inserted {
                Sound::ItemBundleInsert
            } else {
                Sound::ItemBundleInsertFail
            });
            slot.set_stack(other);
            return true;
        }
        if *click == MouseClick::Right && cursor.is_empty() {
            if slot.allow_modification(player)
                && let Some(extracted) = contents.try_extract()
            {
                *cursor = extracted;
                player.play_bundle_sound(Sound::ItemBundleRemoveOne);
            }
            slot.set_stack(other);
            return true;
        }
        contents.selected_item = -1;
        slot.set_stack(other);
    }
    false
}
