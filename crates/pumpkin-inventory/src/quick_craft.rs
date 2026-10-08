use crate::{
    screen_handler::{InventoryPlayer, ScreenHandlerBehaviour},
    slot::Slot,
};
use pumpkin_data::item_stack::ItemStack;

pub fn click(
    behaviour: &mut ScreenHandlerBehaviour,
    index: i32,
    button: i32,
    player: &dyn InventoryPlayer,
) -> Option<(i32, i32)> {
    // AbstractContainerMenu.doClick QUICK_CRAFT (339-404): mode belongs to the start packet.
    let previous = behaviour.drag_status;
    behaviour.drag_status = button & 3;
    let cursor = behaviour
        .cursor_stack
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if ((previous != 1 || behaviour.drag_status != 2) && previous != behaviour.drag_status)
        || cursor.is_empty()
    {
        behaviour.reset_quick_craft();
        return None;
    }
    match behaviour.drag_status {
        0 => {
            behaviour.drag_button = (button >> 2) & 3;
            if behaviour.drag_button <= 1
                || (behaviour.drag_button == 2 && player.has_infinite_materials())
            {
                behaviour.drag_status = 1;
                behaviour.drag_slots.clear();
            } else {
                behaviour.reset_quick_craft();
            }
        }
        1 => {
            if let Some(slot) = usize::try_from(index)
                .ok()
                .and_then(|index| behaviour.slots.get(index))
                && can_replace(slot.as_ref(), &cursor)
                && slot.can_insert(&cursor)
                && (behaviour.drag_button == 2
                    || usize::from(cursor.item_count) > behaviour.drag_slots.len())
            {
                behaviour.drag_slots.insert(index as u32);
            }
        }
        2 => {
            if behaviour.drag_slots.len() == 1 {
                let slot = behaviour.drag_slots.iter().next().copied();
                let mode = behaviour.drag_button;
                behaviour.reset_quick_craft();
                return slot.map(|slot| (slot as i32, mode));
            }
            distribute(behaviour, &cursor);
            behaviour.reset_quick_craft();
        }
        _ => behaviour.reset_quick_craft(),
    }
    None
}

fn can_replace(slot: &dyn Slot, cursor: &ItemStack) -> bool {
    // AbstractContainerMenu.canItemQuickReplace(ignoreSize = true).
    let stack = slot.get_stack();
    stack.is_empty()
        || (stack.are_items_and_components_equal(cursor)
            && stack.item_count <= cursor.get_max_stack_size())
}

fn distribute(behaviour: &ScreenHandlerBehaviour, source: &ItemStack) {
    let slots_count = behaviour.drag_slots.len();
    if slots_count == 0 {
        return;
    }
    let mut remaining = i32::from(source.item_count);
    for index in &behaviour.drag_slots {
        let slot = &behaviour.slots[*index as usize];
        if !can_replace(slot.as_ref(), source)
            || !slot.can_insert(source)
            || (behaviour.drag_button != 2 && usize::from(source.item_count) < slots_count)
        {
            continue;
        }
        let stack = slot.get_stack();
        let carry = if stack.is_empty() {
            0
        } else {
            stack.item_count
        };
        let placed = match behaviour.drag_button {
            0 => usize::from(source.item_count) / slots_count,
            1 => 1,
            _ => usize::from(source.get_max_stack_size()),
        };
        let count = (placed + usize::from(carry))
            .min(usize::from(slot.get_max_item_count_for_stack(source))) as u8;
        remaining -= i32::from(count) - i32::from(carry);
        slot.set_stack(source.copy_with_count(count));
    }
    *behaviour
        .cursor_stack
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        source.copy_with_count(remaining.max(0) as u8);
}

impl ScreenHandlerBehaviour {
    pub(crate) fn reset_quick_craft(&mut self) {
        self.drag_status = 0;
        self.drag_slots.clear();
    }
}
