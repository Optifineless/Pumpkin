#![expect(
    clippy::unwrap_used,
    reason = "Regression fixture requires successful setup"
)]

use super::*;
use crate::entity::EntityBase;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use crate::server::{Server, combat_test_support};
use pumpkin_data::{attributes::Attributes, item::Item};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_protocol::bedrock::{
    client::{CMobArmorEquipment, CMobEquipment, inventory_slot::CInventorySlot},
    network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
    server::inventory_transaction::{WINDOW_ID_ARMOUR, WINDOW_ID_OFF_HAND},
};

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    target: TestBedrockPlayer,
    observer: TestBedrockPlayer,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let target = TestBedrockPlayer::new(&world).await;
        target.player.permission_lvl.store(PermissionLvl::Four);
        let observer = TestBedrockPlayer::new(&world).await;
        observer.player.watched_section.store(
            pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
                target.player.get_entity().chunk_pos.load(),
                std::num::NonZeroU8::new(2).unwrap(),
            ),
        );
        observer
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(target.player.get_entity().chunk_pos.load());
        world.players.store(Arc::new(vec![
            target.player.clone(),
            observer.player.clone(),
        ]));
        world
            .entity_tracker
            .get_tracked_entity(target.player.entity_id())
            .unwrap()
            .update_player(&observer.player, &world);
        assert!(
            world
                .entity_tracker
                .get_tracked_entity(target.player.entity_id())
                .unwrap()
                .seen_by
                .contains(&observer.player.gameprofile.id)
        );
        server.worlds.store(Arc::new(vec![world]));
        target
            .player
            .screen_handler_sync_handler
            .store_player(target.player.clone());
        target
            .player
            .on_screen_handler_opened(&target.player.player_screen_handler);
        Self {
            _directory: directory,
            server,
            target,
            observer,
        }
    }

    fn equip(&self) {
        for (slot, item) in [
            (EquipmentSlot::HEAD, &Item::DIAMOND_HELMET),
            (EquipmentSlot::CHEST, &Item::DIAMOND_CHESTPLATE),
            (EquipmentSlot::LEGS, &Item::DIAMOND_LEGGINGS),
            (EquipmentSlot::FEET, &Item::DIAMOND_BOOTS),
            (EquipmentSlot::OFF_HAND, &Item::SHIELD),
        ] {
            let stack = ItemStack::new(1, item);
            self.target
                .player
                .inventory
                .entity_equipment
                .lock()
                .unwrap()
                .put(&slot, stack.clone());
            self.target.player.enqueue_equipment_change(&slot, &stack);
        }
        self.target.player.tick(&self.server);
    }

    fn clear(&self, input: &str) -> i32 {
        let source = self.target.player.get_command_source(&self.server);
        let count = self
            .server
            .command_dispatcher
            .load()
            .execute_input(input, &source)
            .unwrap();
        self.target.player.tick(&self.server);
        count
    }

    fn empty_slot_packet(&self, container: i32, slot: u32) -> bytes::Bytes {
        self.target
            .client()
            .serialize_packet(&CInventorySlot {
                container_id: (container as u32).into(),
                slot: slot.into(),
                full_container_name: Some(FullContainerName {
                    container_name: if container == WINDOW_ID_ARMOUR {
                        ContainerName::Armor
                    } else {
                        ContainerName::Offhand
                    },
                    dynamic_id: None,
                }),
                storage_item: None,
                item: NetworkItemStackDescriptor::default(),
            })
            .unwrap()
    }
}

#[tokio::test]
async fn clear_bedrock_target_sends_authoritative_armor_and_offhand_inventory() {
    let mut fixture = Fixture::new().await;
    fixture.equip();
    fixture.target.take_packets();
    fixture.observer.take_packets();
    let expected: Vec<_> = (0..4)
        .map(|slot| fixture.empty_slot_packet(WINDOW_ID_ARMOUR, slot))
        .chain(std::iter::once(
            fixture.empty_slot_packet(WINDOW_ID_OFF_HAND, 0),
        ))
        .collect();

    // Counting changes neither storage nor the owner's equipment slots.
    assert_eq!(fixture.clear("clear @s minecraft:diamond_chestplate 0"), 1);
    assert_eq!(
        fixture.target.player.inventory.get_slot(38).item,
        &Item::DIAMOND_CHESTPLATE
    );
    let counted_packets = fixture.target.take_packets();
    assert!(
        expected
            .iter()
            .all(|packet| !counted_packets.contains(packet))
    );

    assert!(fixture.observer.take_packets().is_empty());
    assert_eq!(fixture.clear("clear @s"), 5);
    let runtime_id = (fixture.target.player.entity_id() as u64).into();
    let armor_packet = fixture
        .observer
        .client()
        .serialize_packet(&CMobArmorEquipment {
            target_runtime_id: runtime_id,
            head: NetworkItemStackDescriptor::default(),
            torso: NetworkItemStackDescriptor::default(),
            legs: NetworkItemStackDescriptor::default(),
            feet: NetworkItemStackDescriptor::default(),
            body: NetworkItemStackDescriptor::default(),
        })
        .unwrap();
    let offhand_packet = fixture
        .observer
        .client()
        .serialize_packet(&CMobEquipment {
            target_runtime_id: runtime_id,
            item: NetworkItemStackDescriptor::default(),
            slot: 0,
            selected_slot: 0,
            container_id: WINDOW_ID_OFF_HAND as u8,
        })
        .unwrap();
    let observer_packets = fixture.observer.take_packets();
    assert!(
        observer_packets.contains(&armor_packet),
        "observer lost cleared armor"
    );
    assert!(
        observer_packets.contains(&offhand_packet),
        "observer lost cleared offhand"
    );
    let packets = fixture.target.take_packets();
    for expected_packet in &expected {
        assert!(
            packets.contains(expected_packet),
            "Bedrock target did not receive its cleared armor/offhand inventory slot"
        );
    }
    for slot in 36..=40 {
        assert!(fixture.target.player.inventory.get_slot(slot).is_empty());
    }
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR),
        fixture
            .target
            .player
            .living_entity
            .get_attribute_base(&Attributes::ARMOR)
    );
    fixture.target.player.tick(&fixture.server);
    let idle_packets = fixture.target.take_packets();
    assert!(expected.iter().all(|packet| !idle_packets.contains(packet)));
    fixture.target.close().await;
    fixture.observer.close().await;
}
