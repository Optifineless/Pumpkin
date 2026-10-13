use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::server::packet::PacketSentEvent,
    },
    server::Server,
};
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{
    MultiVersionJavaPacket,
    bedrock::{client::player_hotbar::CPlayerHotbar, server::SMobEquipment},
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

struct MutateBothPublications {
    player: Arc<Player>,
    calls: AtomicUsize,
    direct: bool,
}
impl EventHandler<PacketSentEvent> for MutateBothPublications {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION) {
            match self.calls.fetch_add(1, SeqCst) {
                0 if self.direct => self.player.inventory().set_selected_slot(4),
                0 => self.player.inventory().set_selected_slot(8),
                1 if self.direct => self.player.inventory().set_selected_slot(8),
                1 => self.player.inventory().set_selected_slot(4),
                _ => {}
            }
        }
        Box::pin(async {})
    }
}

async fn publication_reentry(direct: bool) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let stone = ItemStack::new(1, &Item::STONE);
    player.inventory().set_stack(
        4,
        if direct {
            ItemStack::EMPTY.clone()
        } else {
            stone.clone()
        },
    );
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory().set_stack(8, sword.clone());
    let position = BlockPos::new(8, 64, 8);
    fixture.world.world().set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    fixture.drain_packets();
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let callback = Arc::new(MutateBothPublications {
        player: player.clone(),
        calls: AtomicUsize::new(0),
        direct,
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(callback.clone(), EventPriority::Normal, true);
    if direct {
        fixture.owner.client().handle_mob_equipment(
            &fixture.world.server,
            &player,
            &SMobEquipment {
                entity_runtime_id: (player.entity_id() as u64).into(),
                item: NetworkItemStackDescriptor::from(&sword),
                slot: 8,
                selected_slot: 8,
                container_id: 0,
            },
        );
    } else {
        fixture.owner.client().handle_inventory_action(
            &player,
            click_block_packet(&player, HandSlot::Mainhand, position, &stone, Vec::new()),
        );
    }
    player.tick(&fixture.world.server);
    assert_eq!(
        callback.calls.load(SeqCst),
        2,
        "one initial and one corrective equipment publication"
    );
    let selected = if direct { 8 } else { 4 };
    let expected = if direct {
        sword.clone()
    } else {
        ItemStack::EMPTY.clone()
    };
    assert_eq!(player.inventory().get_selected_slot(), selected);
    assert!(player.inventory().held_item().are_equal(&expected));
    assert_eq!(
        round4c::attack_damage(&player, &fixture.java_observer.player),
        if direct { 7.0 } else { 1.0 },
        "a real attack after the terminal callback must use the live hand"
    );
    // The queued final state reaches all recipients on the next tick, without a resend loop.
    player.tick(&fixture.world.server);
    fixture
        .verify_and_close(|fixture| {
            let live = player.inventory().held_item();
            assert_final_publication(fixture, &live, selected, true)
        })
        .await
}

fn assert_final_publication(
    fixture: &mut HandUseFixture,
    live: &ItemStack,
    selected: u8,
    hotbar: bool,
) -> TestResult {
    use crate::net::bedrock::play::inventory_exploit_tests::packet_payload;
    use pumpkin_protocol::{codec::var_ulong::VarULong, serial::PacketRead};
    let owner = fixture.owner.take_packets();
    assert_owner_inventory_slot(
        &owner,
        WINDOW_ID_INVENTORY,
        u32::from(selected),
        live,
        false,
    )?;
    if hotbar {
        let expected = fixture.owner.client().serialize_packet(&CPlayerHotbar {
            selected_slot: VarUInt(u32::from(selected)),
            container_id: 0,
            should_select_slot: true,
        })?;
        let last = owner.iter().rfind(|p| packet_payload(p, 48).is_some());
        assert_eq!(last, Some(&expected), "owner's final selection");
    }
    let entity_id = fixture.owner.player.entity_id();
    let java = CSetEquipment::new(
        entity_id.into(),
        vec![(
            EquipmentSlot::MAIN_HAND.discriminant(),
            ItemStackSerializer::from(live.clone()),
        )],
    );
    let expected = fixture.java_observer.client().serialize_packet(&java)?;
    let version = fixture.java_observer.client().version.load();
    let java_packets = fixture.java_observer.take_packets();
    let last = java_packets.iter().rfind(|p| {
        let mut bytes = p.as_ref();
        VarInt::decode(&mut bytes).is_ok_and(|id| id.0 == CSetEquipment::to_id(version))
            && VarInt::decode(&mut bytes).is_ok_and(|id| id.0 == entity_id)
    });
    assert_eq!(last, Some(&expected), "Java observer's final live hand");
    let bedrock = CMobEquipment {
        target_runtime_id: (entity_id as u64).into(),
        item: NetworkItemStackDescriptor::from(live),
        slot: selected,
        selected_slot: selected,
        container_id: WINDOW_ID_INVENTORY as u8,
    };
    let expected = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock)?;
    let bedrock_packets = fixture.bedrock_observer.take_packets();
    let last = bedrock_packets.iter().rfind(|p| {
        let Some(mut bytes) = packet_payload(p, 31) else {
            return false;
        };
        VarULong::read(&mut bytes).is_ok_and(|id| id.0 == entity_id as u64)
    });
    assert_eq!(last, Some(&expected), "Bedrock observer's final live hand");
    assert_final_attributes(fixture, &owner, &java_packets, &bedrock_packets)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_corrective_publication_preserves_live_damage() -> TestResult {
    publication_reentry(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_corrective_publication_rechecks_item_and_live_damage() -> TestResult {
    publication_reentry(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stacked_equippable_installs_one_and_preserves_displaced_equipment() -> TestResult {
    for hand in [Hand::Right, Hand::Left] {
        for helmet in [
            ItemStack::EMPTY.clone(),
            ItemStack::new(1, &Item::IRON_HELMET),
        ] {
            let mut fixture = HandUseFixture::new().await;
            let player = fixture.owner.player.clone();
            player.inventory().set_selected_slot(4);
            let pumpkins = ItemStack::new(2, &Item::CARVED_PUMPKIN);
            player.inventory().set_stack_in_hand(hand, pumpkins.clone());
            let head_slot = player
                .inventory()
                .equipment_slots
                .iter()
                .find_map(|(slot, equipment)| (*equipment == EquipmentSlot::HEAD).then_some(*slot))
                .ok_or("head inventory slot")?;
            player.inventory().set_stack(head_slot, helmet.clone());
            fixture.drain_packets();
            let mut packet = click_block_packet(
                &player,
                if hand == Hand::Left {
                    HandSlot::Offhand
                } else {
                    HandSlot::Mainhand
                },
                BlockPos::new(8, 64, 8),
                &pumpkins,
                Vec::new(),
            );
            if let TransactionData::UseItem(data) = &mut packet.transaction_data {
                data.action_type = VarInt(1);
            }
            fixture
                .owner
                .client()
                .handle_inventory_action(&player, packet);
            fixture
                .verify_and_close(|_| {
                    assert!(
                        player
                            .inventory()
                            .get_stack(head_slot)
                            .are_equal(&pumpkins.copy_with_count(1))
                    );
                    assert!(
                        player
                            .inventory()
                            .get_stack_in_hand(hand)
                            .are_equal(&pumpkins.copy_with_count(1))
                    );
                    assert_eq!(
                        player.inventory().get_stack_in_hand(hand).uid,
                        pumpkins.uid,
                        "source remainder retains its identity"
                    );
                    assert_ne!(
                        player.inventory().get_stack(head_slot).uid,
                        pumpkins.uid,
                        "equipment split has its own identity"
                    );
                    let main = player.inventory().main_inventory.read().unwrap();
                    assert_eq!(
                        main.iter()
                            .filter(|s| s.are_items_and_components_equal(&helmet))
                            .map(|s| u32::from(s.item_count))
                            .sum::<u32>(),
                        u32::from(!helmet.is_empty())
                    );
                    Ok(())
                })
                .await?;
        }
    }
    Ok(())
}

struct CancelIgnition;
impl EventHandler<crate::plugin::api::events::block::block_ignite::BlockIgniteEvent>
    for CancelIgnition
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::block::block_ignite::BlockIgniteEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_item_use_corrects_both_predicted_blocks() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let tool = ItemStack::new(1, &Item::FLINT_AND_STEEL);
    player.inventory().set_stack(4, tool.clone());
    let position = BlockPos::new(8, 64, 8);
    fixture.world.world().set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    fixture
        .world
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::block::block_ignite::BlockIgniteEvent, _>(
        Arc::new(CancelIgnition),
        EventPriority::Normal,
        true,
    );
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(&player, HandSlot::Mainhand, position, &tool, Vec::new()),
    );
    fixture
        .verify_and_close(|fixture| {
            assert!(player.inventory().held_item().are_equal(&tool));
            assert_eq!(
                fixture.world.world().get_block(&BlockPos::new(8, 65, 8)),
                &Block::AIR
            );
            let packets = fixture.owner.take_packets();
            addendum::assert_block_corrections(fixture, &packets, position)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_predicted_count_does_not_bypass_hand_identity() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let stone = ItemStack::new(2, &Item::STONE);
    player.inventory().set_stack(4, stone.clone());
    let position = BlockPos::new(8, 64, 8);
    fixture.world.world().set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    fixture.drain_packets();
    let mut packet = click_block_packet(&player, HandSlot::Mainhand, position, &stone, Vec::new());
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.item_in_hand = NetworkItemDescriptor::from(&ItemStack::new(1, &Item::DIAMOND));
        data.item_in_hand.stack_size = 0;
    }
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(player.inventory().held_item().are_equal(&stone));
            assert_eq!(
                fixture.world.world().get_block(&BlockPos::new(8, 65, 8)),
                &Block::AIR
            );
            let packets = fixture.owner.take_packets();
            assert!(
                packets.iter().any(|p| {
                    crate::net::bedrock::play::inventory_exploit_tests::packet_payload(p, 49)
                        .is_some()
                }),
                "full hand-identity correction"
            );
            addendum::assert_block_corrections(fixture, &packets, position)
        })
        .await
}

struct ReplacePublishedItem {
    player: Arc<Player>,
    replacement: ItemStack,
    fired: std::sync::atomic::AtomicBool,
}
impl EventHandler<PacketSentEvent> for ReplacePublishedItem {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player
                .inventory()
                .set_stack(8, self.replacement.clone());
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_publication_rechecks_same_slot_value_and_identity() -> TestResult {
    for item in [&Item::IRON_SWORD, &Item::DIAMOND_SWORD] {
        let mut fixture = HandUseFixture::new().await;
        let player = fixture.owner.player.clone();
        player.inventory().set_selected_slot(4);
        let original = ItemStack::new(1, &Item::DIAMOND_SWORD);
        let replacement = ItemStack::new(1, item);
        player.inventory().set_stack(8, original.clone());
        fixture.drain_packets();
        let java = fixture.java_observer.client();
        java.set_player(fixture.java_observer.player.clone());
        java.version.store(JavaMinecraftVersion::V_26_2);
        fixture
            .world
            .server
            .plugin_manager
            .register::<PacketSentEvent, _>(
                Arc::new(ReplacePublishedItem {
                    player: player.clone(),
                    replacement: replacement.clone(),
                    fired: std::sync::atomic::AtomicBool::new(false),
                }),
                EventPriority::Normal,
                true,
            );
        fixture.owner.client().handle_mob_equipment(
            &fixture.world.server,
            &player,
            &SMobEquipment {
                entity_runtime_id: (player.entity_id() as u64).into(),
                item: NetworkItemStackDescriptor::from(&original),
                slot: 8,
                selected_slot: 8,
                container_id: 0,
            },
        );
        player.tick(&fixture.world.server);
        assert_eq!(player.inventory().get_selected_slot(), 8);
        assert_eq!(player.inventory().held_item().uid, replacement.uid);
        assert_eq!(
            round4c::attack_damage(&player, &fixture.java_observer.player),
            if item == &Item::IRON_SWORD { 6.0 } else { 7.0 }
        );
        player.tick(&fixture.world.server);
        fixture
            .verify_and_close(|fixture| {
                let live = player.inventory().held_item();
                assert_final_publication(fixture, &live, 8, false)
            })
            .await?;
    }
    Ok(())
}

fn assert_final_attributes(
    fixture: &HandUseFixture,
    owner: &[bytes::Bytes],
    java_packets: &[bytes::Bytes],
    bedrock_packets: &[bytes::Bytes],
) -> TestResult {
    use crate::net::bedrock::play::inventory_exploit_tests::packet_payload;
    use pumpkin_protocol::{
        bedrock::client::update_attributes::{AttributeData, CUpdateAttributes as BeAttributes},
        codec::var_ulong::VarULong,
        java::client::play::{AttributeModifier, CUpdateAttributes, Property},
        packet::Packet,
        serial::PacketRead,
    };
    let player = &fixture.owner.player;
    let entity_id = player.entity_id();
    let living = &player.living_entity;
    let mut properties = Vec::new();
    for attribute in [&Attributes::ATTACK_DAMAGE, &Attributes::ATTACK_SPEED] {
        let base = living.get_attribute_base(attribute);
        let modifiers = living
            .attributes
            .read()
            .unwrap()
            .get(&attribute.id)
            .map_or_else(Vec::new, |instance| {
                instance
                    .modifiers
                    .iter()
                    .map(|modifier| {
                        AttributeModifier::new(
                            modifier.id.clone(),
                            modifier.amount,
                            modifier.operation as i8,
                        )
                    })
                    .collect()
            });
        properties.push(Property::new(
            VarInt(i32::from(attribute.id)),
            base,
            modifiers,
        ));
    }
    let java = CUpdateAttributes::new(entity_id.into(), properties);
    let expected = fixture.java_observer.client().serialize_packet(&java)?;
    let version = fixture.java_observer.client().version.load();
    let last = java_packets.iter().rfind(|packet| {
        let mut bytes = packet.as_ref();
        VarInt::decode(&mut bytes).is_ok_and(|id| id.0 == CUpdateAttributes::to_id(version))
            && VarInt::decode(&mut bytes).is_ok_and(|id| id.0 == entity_id)
    });
    assert_eq!(
        last,
        Some(&expected),
        "Java final attributes must match the live hand"
    );
    let bedrock = BeAttributes {
        target_runtime_id: (entity_id as u64).into(),
        attribute_list: vec![AttributeData {
            min_value: 0.0,
            max_value: f32::MAX,
            current_value: living.get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32,
            default_min_value: 0.0,
            default_max_value: f32::MAX,
            default_value: living.get_attribute_base(&Attributes::ATTACK_DAMAGE) as f32,
            name: "minecraft:attack_damage".into(),
            modifiers: Vec::new(),
        }],
        tick: 0u64.into(),
    };
    let packet_id = u32::try_from(BeAttributes::PACKET_ID)?;
    for (client, packets) in [
        (fixture.owner.client(), owner),
        (fixture.bedrock_observer.client(), bedrock_packets),
    ] {
        let expected = client.serialize_packet(&bedrock)?;
        let last = packets.iter().rfind(|packet| {
            let Some(mut bytes) = packet_payload(packet, packet_id) else {
                return false;
            };
            VarULong::read(&mut bytes).is_ok_and(|id| id.0 == entity_id as u64)
        });
        assert_eq!(
            last,
            Some(&expected),
            "Bedrock final attributes must match the live hand"
        );
    }
    Ok(())
}
