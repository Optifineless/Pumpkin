use pumpkin_inventory::screen_handler::ScreenHandler;
use pumpkin_protocol::bedrock::{
    network_item::ContainerName,
    server::item_stack_request::{
        ItemStackRequestAction as Action, ItemStackRequestSlotInfo as Slot,
    },
};
use pumpkin_util::GameMode;

use super::item_stack_request::map_bedrock_container_slot;

const fn is_preview(name: ContainerName) -> bool {
    matches!(
        name,
        ContainerName::CraftingOutputPreview
            | ContainerName::AnvilResultPreview
            | ContainerName::GrindstoneResultPreview
            | ContainerName::StonecutterResultPreview
            | ContainerName::LoomResultPreview
            | ContainerName::CartographyResultPreview
            | ContainerName::SmithingTableResultPreview
            | ContainerName::TradeResultPreview
            | ContainerName::Trade2ResultPreview
    )
}

fn identity(handler: &dyn ScreenHandler, slot: &Slot) -> Option<usize> {
    if slot.container_name.container_name == ContainerName::Cursor {
        // Cursor has no physical slot, but every Cursor spelling aliases this one owner.
        Some(handler.get_behaviour().slots.len())
    } else {
        map_bedrock_container_slot(handler, slot.container_name.container_name, slot.slot_id)
            .filter(|index| *index < handler.get_behaviour().slots.len())
    }
}

fn is_physical_preview(handler: &dyn ScreenHandler, slot: &Slot) -> bool {
    use pumpkin_data::screen::WindowType;
    let output = match handler.window_type() {
        None | Some(WindowType::Crafting) => Some(0),
        Some(
            WindowType::Anvil
            | WindowType::Grindstone
            | WindowType::CartographyTable
            | WindowType::Merchant,
        ) => Some(2),
        Some(WindowType::Stonecutter) => Some(1),
        Some(WindowType::Loom | WindowType::Smithing) => Some(3),
        _ => None,
    };
    output.is_some() && identity(handler, slot) == output
}

fn is_unpaid_workstation(handler: &dyn ScreenHandler) -> bool {
    use pumpkin_data::screen::WindowType;
    matches!(
        handler.window_type(),
        Some(
            WindowType::Anvil
                | WindowType::Grindstone
                | WindowType::Loom
                | WindowType::Stonecutter
                | WindowType::Smithing
                | WindowType::Enchantment
        )
    )
}

// ServerGamePacketListenerImpl.handleContainerClick runs menu operations against
// server stock. Predictions never authorize access to a regenerating result slot.
pub(super) fn validate_request(
    handler: &dyn ScreenHandler,
    actions: &[Action],
    mode: GameMode,
) -> bool {
    use pumpkin_data::screen::WindowType;
    // Non-grid output payment is not implemented here; never consume its inputs first.
    if is_unpaid_workstation(handler)
        && actions
            .iter()
            .any(|action| matches!(action, Action::Consume { .. }))
    {
        return false;
    }
    let mut cursor_writes = 0;
    let mut virtual_output = false;
    for action in actions {
        let (source, destination) = match action {
            Action::CraftCreative { .. } => {
                if mode != GameMode::Creative {
                    return false;
                }
                virtual_output = true;
                continue;
            }
            Action::CraftRecipe { .. } | Action::CraftRecipeAuto { .. } => {
                if !matches!(
                    handler.window_type(),
                    None | Some(WindowType::Crafting | WindowType::Merchant)
                ) {
                    return false;
                }
                virtual_output = true;
                continue;
            }
            Action::Take {
                source,
                destination,
                ..
            }
            | Action::Place {
                source,
                destination,
                ..
            } => (Some(source), Some(destination)),
            Action::Swap { slot1, slot2 } => (Some(slot1), Some(slot2)),
            Action::Drop { source, .. }
            | Action::Destroy { source, .. }
            | Action::Consume { source, .. } => (Some(source), None),
            _ => continue,
        };
        let paying_trade = handler.window_type()
            == Some(pumpkin_data::screen::WindowType::Merchant)
            && matches!(action, Action::Take { .. } | Action::Place { .. });
        for (slot, is_source) in source
            .into_iter()
            .map(|slot| (slot, true))
            .chain(destination.into_iter().map(|slot| (slot, false)))
        {
            let name = slot.container_name.container_name;
            let trade_source = paying_trade
                && is_source
                && matches!(
                    name,
                    ContainerName::CreatedOutput
                        | ContainerName::TradeResultPreview
                        | ContainerName::Trade2ResultPreview
                );
            let virtual_source = is_source
                && virtual_output
                && name == ContainerName::CreatedOutput
                && matches!(action, Action::Take { .. } | Action::Place { .. });
            if !trade_source
                && !virtual_source
                && (is_preview(name)
                    || name == ContainerName::CreatedOutput
                    || is_physical_preview(handler, slot))
            {
                return false;
            }
            if !trade_source && !virtual_source && identity(handler, slot).is_none() {
                return false;
            }
            if name == ContainerName::Cursor {
                cursor_writes += 1;
                if cursor_writes > 1 {
                    return false;
                }
            }
        }
        if let (Some(source), Some(destination)) = (source, destination)
            && identity(handler, source).is_some()
            && identity(handler, source) == identity(handler, destination)
        {
            return false;
        }
    }
    true
}
