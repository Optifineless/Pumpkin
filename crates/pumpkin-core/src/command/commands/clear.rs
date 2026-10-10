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
    let mut count: i32 = 0;
    let mut max: i32 = max;
    let mut is_done: bool = false;
    let selected_slot = inventory.get_selected_slot() as usize;
    let mut main_hand_changed = false;
    let mut equipment_changes = Vec::new();

    {
        let mut main_inv = inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (slot_index, slot) in main_inv.iter_mut().enumerate() {
            if test_and_clear(&mut count, &mut max, item, slot, &mut is_done)
                && slot_index == selected_slot
            {
                main_hand_changed = true;
            }
            if is_done {
                break;
            }
        }
    }

    if !is_done {
        let mut entity_equipment_lock = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (equipment_slot, slot) in &mut entity_equipment_lock.equipment {
            if test_and_clear(&mut count, &mut max, item, slot, &mut is_done) {
                equipment_changes.push((equipment_slot.clone(), slot.clone()));
            }
            if is_done {
                break;
            }
        }
    }

    if main_hand_changed {
        equipment_changes.push((EquipmentSlot::MAIN_HAND, inventory.held_item()));
    }

    if !equipment_changes.is_empty() {
        // ClearInventoryCommands relies on LivingEntity.collectEquipmentChanges to publish changed slots.
        target
            .living_entity
            .send_equipment_changes(&equipment_changes);
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
#[expect(
    clippy::unwrap_used,
    reason = "The regression fixture requires successful setup"
)]
mod tests {
    use super::*;
    use crate::entity::EntityBase;
    use crate::net::java::combat_test_support::TestPlayer;
    use crate::server::{Server, combat_test_support};
    use pumpkin_data::attributes::Attributes;
    use pumpkin_data::item::Item;
    use pumpkin_data::packet::{CURRENT_MC_VERSION, clientbound::play::SET_EQUIPMENT};
    use pumpkin_protocol::{
        ServerPacket, codec::var_int::VarInt, java::client::play::CSetEquipment,
    };
    use pumpkin_util::PermissionLvl;
    use std::sync::Arc;

    struct ClearFixture {
        _directory: tempfile::TempDir,
        server: Arc<Server>,
        target: TestPlayer,
        observer: TestPlayer,
    }

    impl ClearFixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let server = combat_test_support::server(directory.path());
            let world = combat_test_support::world(&server, directory.path());
            let target = TestPlayer::new(&world);
            let observer = TestPlayer::new(&world);
            target.player.permission_lvl.store(PermissionLvl::Four);
            server.worlds.store(Arc::new(vec![world.clone()]));
            world.players.store(Arc::new(vec![
                target.player.clone(),
                observer.player.clone(),
            ]));
            observer
                .player
                .chunk_sender
                .lock()
                .unwrap()
                .mark_sent_out_of_band(target.player.get_entity().chunk_pos.load());
            world
                .entity_tracker
                .add_entity(&(target.player.clone() as Arc<dyn EntityBase>), &world);
            Self {
                _directory: directory,
                server,
                target,
                observer,
            }
        }

        fn equip_diamond_armor(&self) {
            let equipment = [
                (
                    EquipmentSlot::HEAD,
                    ItemStack::new(1, &Item::DIAMOND_HELMET),
                ),
                (
                    EquipmentSlot::CHEST,
                    ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
                ),
                (
                    EquipmentSlot::LEGS,
                    ItemStack::new(1, &Item::DIAMOND_LEGGINGS),
                ),
                (EquipmentSlot::FEET, ItemStack::new(1, &Item::DIAMOND_BOOTS)),
            ];
            {
                let mut stored = self
                    .target
                    .player
                    .inventory
                    .entity_equipment
                    .lock()
                    .unwrap();
                for (slot, stack) in &equipment {
                    stored.put(slot, stack.clone());
                }
            }
            self.target
                .player
                .living_entity
                .send_equipment_changes(&equipment);
        }

        fn run_clear(&self, input: &str) -> Result<i32, CommandSyntaxError> {
            let source = self.target.player.get_command_source(&self.server);
            self.server
                .command_dispatcher
                .load()
                .execute_input(input, &source)
        }

        fn take_observer_equipment_updates(&mut self) -> Vec<CSetEquipment> {
            self.observer
                .take_packets()
                .into_iter()
                .filter_map(|packet| {
                    let mut packet = packet.as_ref();
                    let packet_id = VarInt::decode(&mut packet).ok()?.0;
                    (packet_id == SET_EQUIPMENT.0)
                        .then(|| CSetEquipment::read(&mut packet, &CURRENT_MC_VERSION).ok())
                        .flatten()
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn clear_removes_diamond_armor_attribute_modifiers() {
        let fixture = ClearFixture::new();
        fixture.equip_diamond_armor();
        let armor_base = fixture
            .target
            .player
            .living_entity
            .get_attribute_base(&Attributes::ARMOR);
        assert!(
            fixture
                .target
                .player
                .living_entity
                .get_attribute_value(&Attributes::ARMOR)
                > armor_base
        );

        assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);
        assert_eq!(
            fixture
                .target
                .player
                .living_entity
                .get_attribute_value(&Attributes::ARMOR),
            armor_base
        );
    }

    #[tokio::test]
    async fn clear_sends_empty_armor_slots_to_tracking_players() {
        let mut fixture = ClearFixture::new();
        fixture.equip_diamond_armor();
        fixture.target.take_packets();
        fixture.observer.take_packets();

        assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);
        let updates = fixture.take_observer_equipment_updates();
        let armor_slots = [
            EquipmentSlot::HEAD,
            EquipmentSlot::CHEST,
            EquipmentSlot::LEGS,
            EquipmentSlot::FEET,
        ];
        let empty_slots: Vec<_> = updates
            .iter()
            .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
            .flat_map(|update| {
                update
                    .equipment
                    .iter()
                    .filter(|(_, stack)| stack.0.is_empty())
                    .map(|(slot, _)| *slot)
            })
            .collect();
        for slot in armor_slots {
            assert!(
                empty_slots.contains(&slot.discriminant()),
                "tracking player did not receive empty equipment slot {}",
                slot.discriminant()
            );
        }
    }

    #[tokio::test]
    async fn clear_matching_held_item_sends_empty_main_hand_to_tracking_players() {
        let mut fixture = ClearFixture::new();
        fixture
            .target
            .player
            .inventory
            .set_held_item(ItemStack::new(1, &Item::DIAMOND_SWORD));
        fixture.target.take_packets();
        fixture.observer.take_packets();

        assert_eq!(
            fixture
                .run_clear("clear @s minecraft:diamond_sword")
                .unwrap(),
            1
        );
        assert!(fixture.target.player.inventory.held_item().is_empty());
        let updates = fixture.take_observer_equipment_updates();
        assert!(updates.iter().any(|update| {
            update.entity_id.0 == fixture.target.player.entity_id()
                && update.equipment.iter().any(|(slot, stack)| {
                    *slot == EquipmentSlot::MAIN_HAND.discriminant() && stack.0.is_empty()
                })
        }));
    }
}
