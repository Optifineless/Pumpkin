use std::sync::Arc;

use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::core::integer::IntegerArgumentType;
use crate::command::argument_types::entity::EntityArgumentType;
use crate::command::argument_types::item_predicate::{ItemPredicate, ItemPredicateArgumentType};
use crate::command::context::command_context::CommandContext;
use crate::command::context::command_source::CommandSource;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::entity::player::Player;

const DESCRIPTION: &str = "Clear your inventory or that of target(s).";
const PERMISSION: &str = "minecraft:command.clear";

const ERROR_SINGLE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::CLEAR_FAILED_SINGLE,
    translation::java::CLEAR_FAILED_SINGLE,
);

const ERROR_MULTIPLE: CommandErrorType<1> = CommandErrorType::new(
    translation::java::CLEAR_FAILED_MULTIPLE,
    translation::java::CLEAR_FAILED_MULTIPLE,
);

const MAX_NO_UPPER_LIMIT: i32 = -1;
const MAX_NO_CLEAR_BUT_SIMULATE: i32 = 0;

fn clear_player(target: &Player, item: &ItemPredicate, max: i32) -> i32 {
    let inventory = target.inventory();
    let owner = pumpkin_inventory::Inventory::slot_transaction(inventory.as_ref(), 0);
    let mut count: i32 = 0;
    let mut max: i32 = max;
    let mut is_done: bool = false;
    let mut inventory_changed = false;
    let mut equipment_slots = Vec::new();

    #[cfg(test)]
    review_tests::run_hook(review_tests::HookPoint::BeforeInventoryScan, target);
    let guard = owner.lock();
    {
        let mut main_inv = inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (slot_index, slot) in main_inv.iter_mut().enumerate() {
            if test_and_clear(&mut count, &mut max, item, slot, &mut is_done) {
                inventory_changed = true;
                // PlayerEquipment.get(MAINHAND) reads the selected slot at detection time.
                if pumpkin_inventory::player::player_inventory::PlayerInventory::is_valid_hotbar_index(slot_index)
                    && !equipment_slots.contains(&EquipmentSlot::MAIN_HAND)
                {
                    equipment_slots.push(EquipmentSlot::MAIN_HAND);
                }
            }
            if is_done {
                break;
            }
        }
    }

    if !is_done {
        let mut equipment = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (equipment_slot, slot) in &mut equipment.equipment {
            if test_and_clear(&mut count, &mut max, item, slot, &mut is_done) {
                inventory_changed = true;
                equipment_slots.push(equipment_slot.clone());
            }
            if is_done {
                break;
            }
        }
    }

    drop(guard);
    #[cfg(test)]
    review_tests::run_hook(review_tests::HookPoint::BeforeEquipmentUpdates, target);
    // ClearInventoryCommands.clearInventory mutates slots; LivingEntity.detectEquipmentUpdates
    // reads their current contents on the tick. Queue slots, never cleared-stack snapshots.
    target.queue_equipment_updates(&equipment_slots);
    if inventory_changed {
        // ClearInventoryCommands.clearInventory broadcasts changed slots to both open menus.
        target.sync_inventory_to_client();
    }

    count
}

fn test_and_clear(
    count: &mut i32,
    max: &mut i32,
    item: &ItemPredicate,
    slot_lock: &mut ItemStack,
    is_done: &mut bool,
) -> bool {
    if item.test(slot_lock) {
        let item_count = slot_lock.item_count as i32;
        if *max == MAX_NO_CLEAR_BUT_SIMULATE {
            *count += item_count;
            return false;
        }
        if *max == MAX_NO_UPPER_LIMIT {
            *count += item_count;
            if slot_lock.are_equal(ItemStack::EMPTY) {
                return false;
            }
            *slot_lock = ItemStack::EMPTY.clone();
            return true;
        }

        let taken = i32::min(*max, item_count);
        *count += taken;
        if taken == 0 {
            if slot_lock.are_equal(ItemStack::EMPTY) {
                return false;
            }
            *slot_lock = ItemStack::EMPTY.clone();
            return true;
        }
        if taken == item_count {
            *slot_lock = ItemStack::EMPTY.clone();
        } else {
            slot_lock.decrement(taken as u8);
        }
        *max -= taken;
        *is_done = *max == 0;
        return true;
    }

    false
}

fn clear_inventory(
    source: &CommandSource,
    players: &[Arc<Player>],
    predicate: &ItemPredicate,
    max_count: i32,
) -> Result<i32, CommandSyntaxError> {
    let mut total_count = 0;

    for player in players {
        total_count += clear_player(player, predicate, max_count);
    }

    if total_count == 0 {
        if let [first_players] = players {
            let player_name = first_players.gameprofile.name.clone();
            Err(ERROR_SINGLE.create_without_context(TextComponent::text(player_name)))
        } else {
            Err(ERROR_MULTIPLE
                .create_without_context(TextComponent::text(players.len().to_string())))
        }
    } else {
        if max_count == 0 {
            if let [first_players] = players {
                let player_name = first_players.gameprofile.name.clone();
                source.send_feedback(
                    TextComponent::translate_cross(
                        translation::java::COMMANDS_CLEAR_TEST_SINGLE,
                        translation::java::COMMANDS_CLEAR_TEST_SINGLE,
                        [
                            TextComponent::text(total_count.to_string()),
                            TextComponent::text(player_name),
                        ],
                    ),
                    true,
                );
            } else {
                source.send_feedback(
                    TextComponent::translate_cross(
                        translation::java::COMMANDS_CLEAR_TEST_MULTIPLE,
                        translation::java::COMMANDS_CLEAR_TEST_MULTIPLE,
                        [
                            TextComponent::text(total_count.to_string()),
                            TextComponent::text(players.len().to_string()),
                        ],
                    ),
                    true,
                );
            }
        } else if let [first_players] = players {
            let player_name = first_players.gameprofile.name.clone();
            source.send_feedback(
                TextComponent::translate_cross(
                    translation::java::COMMANDS_CLEAR_SUCCESS_SINGLE,
                    translation::java::COMMANDS_CLEAR_SUCCESS_SINGLE,
                    [
                        TextComponent::text(total_count.to_string()),
                        TextComponent::text(player_name),
                    ],
                ),
                true,
            );
        } else {
            source.send_feedback(
                TextComponent::translate_cross(
                    translation::java::COMMANDS_CLEAR_SUCCESS_MULTIPLE,
                    translation::java::COMMANDS_CLEAR_SUCCESS_MULTIPLE,
                    [
                        TextComponent::text(total_count.to_string()),
                        TextComponent::text(players.len().to_string()),
                    ],
                ),
                true,
            );
        }

        Ok(total_count)
    }
}

#[derive(Clone, Copy)]
enum ClearStep {
    CallerOnly,
    TargetsOnly,
    WithItem,
    WithMaxCount,
}

struct ClearExecutor {
    step: ClearStep,
}

impl CommandExecutor for ClearExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        // ClearInventoryCommands.register resolves implicit targets through getPlayerOrException().
        match self.step {
            ClearStep::CallerOnly => {
                let player = context.source.player_arc_or_err()?;
                clear_inventory(
                    &context.source,
                    std::slice::from_ref(&player),
                    &ItemPredicate::Any,
                    -1,
                )
            }
            ClearStep::TargetsOnly => {
                let targets = EntityArgumentType::get_players(context, "targets")?;
                clear_inventory(&context.source, &targets, &ItemPredicate::Any, -1)
            }
            ClearStep::WithItem => {
                let targets = EntityArgumentType::get_players(context, "targets")?;
                let item = ItemPredicateArgumentType::get(context, "item")?;
                clear_inventory(&context.source, &targets, &item, -1)
            }
            ClearStep::WithMaxCount => {
                let targets = EntityArgumentType::get_players(context, "targets")?;
                let item = ItemPredicateArgumentType::get(context, "item")?;
                let max_count = IntegerArgumentType::get(context, "maxCount")?;
                clear_inventory(&context.source, &targets, &item, max_count)
            }
        }
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("clear", DESCRIPTION)
            .requires(PERMISSION)
            .executes(ClearExecutor {
                step: ClearStep::CallerOnly,
            })
            .then(
                argument("targets", EntityArgumentType::Players)
                    .executes(ClearExecutor {
                        step: ClearStep::TargetsOnly,
                    })
                    .then(
                        argument("item", ItemPredicateArgumentType)
                            .executes(ClearExecutor {
                                step: ClearStep::WithItem,
                            })
                            .then(
                                argument("maxCount", IntegerArgumentType::with_min(0)).executes(
                                    ClearExecutor {
                                        step: ClearStep::WithMaxCount,
                                    },
                                ),
                            ),
                    ),
            ),
    );
}

#[cfg(test)]
mod bedrock_owner_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod bedrock_observer_tests;

#[cfg(test)]
mod fresh_control_tests;
