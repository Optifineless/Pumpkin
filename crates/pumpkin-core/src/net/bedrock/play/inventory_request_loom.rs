use std::sync::Arc;

use pumpkin_data::{data_component_impl::BannerPatternsImpl, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory, loom_screen_handler::LoomScreenHandler, screen_handler::ScreenHandler,
};
use pumpkin_protocol::bedrock::{
    banner_pattern::java_pattern,
    network_item::{ContainerName, FullContainerName},
    server::item_stack_request::{
        ItemStackRequest, ItemStackRequestAction as Action, ItemStackRequestSlotInfo as Slot,
    },
};

use super::item_stack_request::{SlotUpdate, map_bedrock_container_slot, record_update};
use crate::entity::player::Player;

pub(super) const BEDROCK_INPUT_SLOT: u8 = 9;
pub(super) const BEDROCK_DYE_SLOT: u8 = 10;
const BEDROCK_RESULT_SLOT: u8 = 50;

struct Selection<'a> {
    pattern: &'a str,
    repetitions: u8,
    source: &'a Slot,
    destination: &'a Slot,
    consumes: [Option<&'a Slot>; 2],
}

impl<'a> Selection<'a> {
    fn parse(request: &'a ItemStackRequest) -> Option<Self> {
        let (first, suffix) = request.actions.split_first()?;
        let Action::Loom {
            pattern_id,
            repetitions,
        } = first
        else {
            return None;
        };
        if *repetitions == 0 {
            return None;
        }
        // Geyser's LoomInventoryTranslator receives result prediction immediately
        // after CraftLoom. Its item data never authorizes or supplies our output.
        let suffix = if let Some((
            Action::CraftResultsDeprecated {
                result_items,
                times_crafted,
            },
            rest,
        )) = suffix.split_first()
        {
            if result_items.len() != 1 || times_crafted != repetitions {
                return None;
            }
            rest
        } else {
            suffix
        };
        let mut output = None;
        let mut consumes = [None, None];
        for action in suffix {
            match action {
                Action::Take {
                    count,
                    source,
                    destination,
                }
                | Action::Place {
                    count,
                    source,
                    destination,
                } if *count == *repetitions
                    && output.is_none()
                    && source.slot_id == BEDROCK_RESULT_SLOT
                    && matches!(
                        source.container_name.container_name,
                        ContainerName::CreatedOutput
                            | ContainerName::LoomResultPreview
                            | ContainerName::CraftingOutputPreview
                    ) =>
                {
                    output = Some((source, destination));
                }
                Action::Consume { count, source } if *count == *repetitions => {
                    let index = match source.container_name.container_name {
                        ContainerName::LoomInput if source.slot_id == BEDROCK_INPUT_SLOT => 0,
                        ContainerName::LoomDye if source.slot_id == BEDROCK_DYE_SLOT => 1,
                        _ => return None,
                    };
                    if consumes[index].replace(source).is_some() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        if consumes[0].is_some() != consumes[1].is_some() {
            return None;
        }
        let (source, destination) = output?;
        Some(Self {
            pattern: pattern_id,
            repetitions: *repetitions,
            source,
            destination,
            consumes,
        })
    }
}

const fn matches_stack(info: &Slot, stack: &ItemStack) -> bool {
    info.container_name.dynamic_id.is_none()
        && info.stack_id == if stack.is_empty() { 0 } else { stack.uid.get() }
}

fn destination_stack(handler: &dyn ScreenHandler, destination: &Slot) -> Option<ItemStack> {
    if destination.container_name.dynamic_id.is_some() {
        return None;
    }
    match destination.container_name.container_name {
        ContainerName::Cursor => Some(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        ),
        ContainerName::Inventory
        | ContainerName::CombinedHotBarAndInventory
        | ContainerName::HotBar => {
            if destination.slot_id
                >= if destination.container_name.container_name == ContainerName::HotBar {
                    9
                } else {
                    36
                }
            {
                return None;
            }
            let index = map_bedrock_container_slot(
                handler,
                destination.container_name.container_name,
                destination.slot_id,
            )?;
            Some(handler.get_behaviour().slots.get(index)?.get_cloned_stack())
        }
        _ => None,
    }
}

// LoomMenu.clickMenuButton -> setupResultSlot computes the preview on isolated
// inputs. Only the live result slot's onTake is allowed to pay for a banner.
fn preview(
    loom: &LoomScreenHandler,
    player: &Arc<Player>,
    selection: &Selection<'_>,
    predictions: &super::inventory_request_predictions::Results,
) -> Option<(usize, ItemStack)> {
    let inputs = (0..3)
        .map(|index| loom.input_inventory.get_stack(index))
        .collect::<Vec<_>>();
    for (index, input) in inputs.iter().take(2).enumerate() {
        if input.is_empty()
            || input.item_count < selection.repetitions
            || !loom.get_behaviour().slots[index].can_insert(input)
            || !loom.get_behaviour().slots[index].can_take_stack(player.as_ref(), input)
            || selection.consumes[index]
                .is_some_and(|source| predictions.resolve(source) != Some(input.uid.get()))
        {
            return None;
        }
    }
    if (!inputs[2].is_empty() && !loom.get_behaviour().slots[2].can_insert(&inputs[2]))
        || inputs[0]
            .get_data_component::<BannerPatternsImpl>()
            .is_some_and(|patterns| patterns.layers.len() >= 6)
    {
        return None;
    }

    let mut staged = LoomScreenHandler::new(loom.sync_id(), &player.inventory);
    for (index, input) in inputs.into_iter().enumerate() {
        staged.input_inventory.set_stack(index, input);
    }
    staged.slots_changed();
    let pattern = java_pattern(selection.pattern)?;
    let index = staged
        .selectable_patterns
        .iter()
        .position(|key| key.strip_prefix("minecraft:") == Some(pattern))?;
    if !staged.on_button_click(player.as_ref(), i32::try_from(index).ok()?) {
        return None;
    }
    let output = staged.output_inventory.get_stack(0);
    (!output.is_empty() && loom.get_behaviour().slots[3].can_take_stack(player.as_ref(), &output))
        .then_some((index, output))
}

pub(super) fn apply(
    handler: &mut dyn ScreenHandler,
    player: &Arc<Player>,
    request: &ItemStackRequest,
    updates: &mut Vec<SlotUpdate>,
    predictions: &super::inventory_request_predictions::Results,
) -> bool {
    let Some(selection) = Selection::parse(request) else {
        return false;
    };
    let Some(loom) = handler.as_any_mut().downcast_mut::<LoomScreenHandler>() else {
        return false;
    };
    let Some((index, output)) = preview(loom, player, &selection, predictions) else {
        return false;
    };
    // CreatedOutput references this request's generated batch. Preview aliases
    // reference the authoritative preview identity, never another owned stack.
    let source_id =
        if selection.source.container_name.container_name == ContainerName::CreatedOutput {
            if request.request_id.0 >= 0 {
                return false;
            }
            request.request_id.0
        } else {
            let live = loom.output_inventory.get_stack(0);
            if live.is_empty() || !live.are_items_and_components_equal(&output) {
                return false;
            }
            live.uid.get()
        };
    if selection.source.container_name.dynamic_id.is_some()
        || selection.source.stack_id != source_id
    {
        return false;
    }
    let Some(expected) = destination_stack(loom, selection.destination) else {
        return false;
    };
    if !matches_stack(selection.destination, &expected) {
        return false;
    }
    let stock = output.copy_with_count(selection.repetitions);
    let Some(replacement) = super::inventory_commit::virtual_destination(
        loom,
        selection.destination,
        &stock,
        &expected,
        selection.repetitions,
    ) else {
        return false;
    };

    // All backing owners and the current menu identity remain held by the
    // dispatcher. No fallible operation remains after this selection/debit.
    loom.slots_changed();
    loom.on_button_click(player.as_ref(), index as i32);
    let result = loom.get_behaviour().slots[3].clone();
    for _ in 0..selection.repetitions {
        // Player's InventoryPlayer implementation queues the statistic callback
        // through GuardEffects, after every menu/inventory owner has released.
        result.safe_take(1, 1, player.as_ref());
    }
    if selection.destination.container_name.container_name == ContainerName::Cursor {
        *loom
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = replacement.clone();
    } else if let Some(index) = map_bedrock_container_slot(
        loom,
        selection.destination.container_name.container_name,
        selection.destination.slot_id,
    ) {
        super::inventory_commit::queue_equipment_change(player, loom, index, &replacement);
        loom.get_behaviour().slots[index].set_stack(replacement.clone());
    }
    record_update(
        updates,
        selection.destination.container_name.clone(),
        selection.destination.slot_id,
        &replacement,
    );
    record_outputs(loom, &selection, updates);
    true
}

fn record_outputs(
    loom: &LoomScreenHandler,
    selection: &Selection<'_>,
    updates: &mut Vec<SlotUpdate>,
) {
    // Geyser LoomInventoryTranslator.javaSlotToBedrockContainer UI addresses.
    for (index, (name, slot_id)) in [
        (ContainerName::LoomInput, BEDROCK_INPUT_SLOT),
        (ContainerName::LoomDye, BEDROCK_DYE_SLOT),
        (ContainerName::LoomMaterial, 11),
        (ContainerName::LoomResultPreview, BEDROCK_RESULT_SLOT),
    ]
    .into_iter()
    .enumerate()
    {
        record_update(
            updates,
            FullContainerName {
                container_name: name,
                dynamic_id: None,
            },
            slot_id,
            &loom.get_behaviour().slots[index].get_cloned_stack(),
        );
    }
    let source_stack =
        if selection.source.container_name.container_name == ContainerName::CreatedOutput {
            ItemStack::EMPTY.clone()
        } else {
            loom.output_inventory.get_stack(0)
        };
    record_update(
        updates,
        selection.source.container_name.clone(),
        selection.source.slot_id,
        &source_stack,
    );
    // CreatedOutput is exhausted even while the physical next preview is full.
    record_update(
        updates,
        FullContainerName {
            container_name: ContainerName::CreatedOutput,
            dynamic_id: None,
        },
        BEDROCK_RESULT_SLOT,
        ItemStack::EMPTY,
    );
}
