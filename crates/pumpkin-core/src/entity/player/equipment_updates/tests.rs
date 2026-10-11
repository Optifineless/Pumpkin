use super::*;
use crate::entity::EntityBase;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use crate::net::java::combat_test_support::TestPlayer;
use crate::server::{Server, combat_test_support};
use pumpkin_data::packet::{CURRENT_MC_VERSION, clientbound::play::SET_EQUIPMENT};
use pumpkin_data::{
    attributes::Attributes, data_component_impl::AttributeModifiersImpl, item::Item,
};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_protocol::{
    ServerPacket,
    bedrock::{
        network_item::{ContainerName, FullContainerName},
        server::{
            inventory_transaction::{
                InventoryAction, NormalTransactionData, SInventoryTransaction, TransactionData,
            },
            item_stack_request::{
                ItemStackRequest, ItemStackRequestAction, ItemStackRequestSlotInfo,
                SItemStackRequest,
            },
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    java::{client::play::CSetEquipment, server::play::SSetCreativeSlot},
};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum PacketRoute {
    Creative,
    InventoryAction,
    StackRequest,
}

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    target: TestPlayer,
    observer: TestPlayer,
    bedrock: TestBedrockPlayer,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let target = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        let bedrock = TestBedrockPlayer::new(&world).await;
        server.worlds.store(Arc::new(vec![world.clone()]));
        world.players.store(Arc::new(vec![
            target.player.clone(),
            observer.player.clone(),
            bedrock.player.clone(),
        ]));
        for player in [&target.player, &bedrock.player] {
            observer
                .player
                .chunk_sender
                .lock()
                .unwrap()
                .mark_sent_out_of_band(player.get_entity().chunk_pos.load());
            world
                .entity_tracker
                .get_tracked_entity(player.entity_id())
                .unwrap()
                .update_player(&observer.player, &world);
        }
        Self {
            _directory: directory,
            server,
            target,
            observer,
            bedrock,
        }
    }

    fn player(&self, route: PacketRoute) -> &Arc<Player> {
        match route {
            PacketRoute::Creative => &self.target.player,
            PacketRoute::InventoryAction | PacketRoute::StackRequest => &self.bedrock.player,
        }
    }

    fn write_packet(&self, route: PacketRoute, slot: u8, stack: ItemStack) {
        let player = self.player(route);
        match route {
            PacketRoute::Creative => {
                self.target
                    .client()
                    .handle_set_creative_slot(
                        player,
                        SSetCreativeSlot::new(i16::from(slot) + 36, stack.into()),
                    )
                    .unwrap();
            }
            PacketRoute::InventoryAction => {
                self.bedrock.client().handle_inventory_action(
                    player,
                    SInventoryTransaction {
                        legacy_request_id: VarInt(0),
                        legacy_set_item_slots: Vec::new(),
                        has_value: true,
                        actions: vec![InventoryAction {
                            source_type: 0,
                            window_id: Some(0),
                            source_flags: None,
                            inventory_slot: u32::from(slot),
                            old_item: (&player.inventory.get_slot(usize::from(slot))).into(),
                            new_item: (&stack).into(),
                        }],
                        transaction_type: VarUInt(0),
                        transaction_data: TransactionData::Normal(NormalTransactionData),
                    },
                );
            }
            PacketRoute::StackRequest => {
                // Swap the selected item with a real second inventory slot, including EMPTY.
                player.inventory.set_slot(9, stack);
                self.bedrock.client().handle_item_stack_request(
                    player,
                    SItemStackRequest {
                        requests: vec![ItemStackRequest {
                            request_id: VarInt(1),
                            actions: vec![ItemStackRequestAction::Swap {
                                slot1: request_slot(slot),
                                slot2: request_slot(9),
                            }],
                            filter_strings: Vec::new(),
                            filter_cause: 0,
                        }],
                    },
                );
            }
        }
    }

    fn take_equipment(&mut self, player: &Player) -> Vec<ItemStack> {
        self.observer
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut data = packet.as_ref();
                (VarInt::decode(&mut data).ok()?.0 == SET_EQUIPMENT.0)
                    .then(|| CSetEquipment::read(&mut data, &CURRENT_MC_VERSION).ok())
                    .flatten()
            })
            .filter(|update| update.entity_id.0 == player.entity_id())
            .flat_map(|update| update.equipment)
            .filter(|(slot, _)| *slot == EquipmentSlot::MAIN_HAND.discriminant())
            .map(|(_, stack)| stack.0.into_owned())
            .collect()
    }
}

fn request_slot(slot_id: u8) -> ItemStackRequestSlotInfo {
    ItemStackRequestSlotInfo {
        container_name: FullContainerName {
            container_name: ContainerName::Inventory,
            dynamic_id: None,
        },
        slot_id,
        stack_id: 0,
    }
}

fn assert_attack_modifiers(player: &Player, stack: &ItemStack) {
    let component = stack.get_data_component::<AttributeModifiersImpl>();
    for attribute in [&Attributes::ATTACK_DAMAGE, &Attributes::ATTACK_SPEED] {
        let expected: Vec<_> = component
            .iter()
            .flat_map(|component| component.attribute_modifiers.iter())
            .filter(|modifier| modifier.r#type == attribute)
            .map(|modifier| (modifier.id, modifier.amount))
            .collect();
        let attributes = player.living_entity.attributes.read().unwrap();
        let actual: Vec<_> = attributes[&attribute.id]
            .modifiers
            .iter()
            .map(|modifier| (modifier.id.as_str(), modifier.amount))
            .collect();
        assert_eq!(actual, expected);
        drop(attributes);
        if stack.is_empty() {
            assert_eq!(
                player.living_entity.get_attribute_value(attribute),
                player.living_entity.get_attribute_base(attribute)
            );
        }
    }
}

async fn packet_writes_publish_current_hand(route: PacketRoute) {
    let mut fixture = Fixture::new().await;
    let player = fixture.player(route).clone();
    player.set_gamemode(pumpkin_util::GameMode::Creative);
    player.inventory.set_selected_slot(3);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory.set_held_item(sword.clone());
    player.enqueue_equipment_change(&EquipmentSlot::MAIN_HAND, &sword);
    player.tick(&fixture.server);
    assert_attack_modifiers(&player, &sword);
    fixture.observer.take_packets();

    // An unselected hotbar write must leave the selected sword and its modifiers alone.
    fixture.write_packet(route, 4, ItemStack::new(1, &Item::STONE));
    player.tick(&fixture.server);
    assert!(fixture.take_equipment(&player).is_empty());
    assert!(player.inventory.held_item().are_equal(&sword));
    assert_attack_modifiers(&player, &sword);

    for replacement in [
        ItemStack::new(1, &Item::IRON_SWORD),
        ItemStack::EMPTY.clone(),
    ] {
        fixture.write_packet(route, 3, replacement.clone());
        assert!(
            fixture.take_equipment(&player).is_empty(),
            "notification published before the packet completed"
        );
        player.tick(&fixture.server);
        assert!(player.inventory.held_item().are_equal(&replacement));
        let updates = fixture.take_equipment(&player);
        assert!(
            updates
                .last()
                .is_some_and(|stack| stack.are_equal(&replacement)),
            "observer's last main hand was not the completed packet write"
        );
        assert_attack_modifiers(&player, &replacement);
        player.tick(&fixture.server);
        assert!(fixture.take_equipment(&player).is_empty());
    }
    fixture.bedrock.close().await;
}

#[tokio::test]
async fn creative_packet_publishes_replacement_and_removed_selected_sword() {
    packet_writes_publish_current_hand(PacketRoute::Creative).await;
}

#[tokio::test]
async fn bedrock_inventory_action_publishes_replacement_and_removed_selected_sword() {
    packet_writes_publish_current_hand(PacketRoute::InventoryAction).await;
}

#[tokio::test]
async fn bedrock_stack_request_publishes_replacement_and_removed_selected_sword() {
    packet_writes_publish_current_hand(PacketRoute::StackRequest).await;
}

#[tokio::test]
async fn panicking_equipment_publication_does_not_block_later_player_ticks() {
    let mut fixture = Fixture::new().await;
    let player = fixture.target.player.clone();
    player.set_gamemode(pumpkin_util::GameMode::Creative);
    fixture.write_packet(
        PacketRoute::Creative,
        0,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
    );
    Player::set_equipment_update_test_hook(|_| panic!("publication hook"));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| player.tick(&fixture.server)))
            .is_err()
    );

    let replacement = ItemStack::new(1, &Item::IRON_SWORD);
    fixture.write_packet(PacketRoute::Creative, 0, replacement.clone());
    player.tick(&fixture.server);
    let updates = fixture.take_equipment(&player);
    assert!(
        updates
            .last()
            .is_some_and(|stack| stack.are_equal(&replacement))
    );
    assert_attack_modifiers(&player, &replacement);
    fixture.bedrock.close().await;
}
