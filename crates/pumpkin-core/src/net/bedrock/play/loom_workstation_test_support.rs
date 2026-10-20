#![expect(
    clippy::unwrap_used,
    clippy::unreachable,
    reason = "Real workstation regression fixtures"
)]

use std::io::Cursor;

use bytes::Bytes;
use pumpkin_data::{
    Block,
    biome::Biome,
    data_component_impl::{BannerPatternsImpl, CustomNameImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::{
    Inventory, loom_screen_handler::LoomScreenHandler, screen_handler::SharedScreenHandler,
};
use pumpkin_nbt::{Nbt, NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    bedrock::{
        client::{
            inventory_content::CInventoryContent, inventory_slot::CInventorySlot,
            item_stack_response::CItemStackResponse,
        },
        network_item::{ContainerName, FullContainerName, NetworkItemDescriptor},
        server::{
            container_close::SContainerClose,
            inventory_transaction::{
                HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
            },
            item_stack_request::{
                ItemStackRequest, ItemStackRequestAction as Action,
                ItemStackRequestSlotInfo as RequestSlot, SItemStackRequest, StackRequestItem,
            },
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    packet::Packet,
    serial::{PacketRead, PacketWrite},
};
use pumpkin_util::{Hand, math::position::BlockPos, math::vector3::Vector3};

use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::bedrock::combat_test_support::TestBedrockPlayer,
    world::spawn_test_support::{proto, publish},
};

pub(in crate::net::bedrock::play) const POSITION: BlockPos = BlockPos::new(8, 64, 8);

pub(in crate::net::bedrock::play) struct LoomFixture {
    pub world: DeathTestWorld,
    pub bedrock: TestBedrockPlayer,
    pub screen: SharedScreenHandler,
    pub last_packets: Vec<Bytes>,
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::net::bedrock::play) struct ResponseSlot {
    pub name: ContainerName,
    pub slot: u8,
    pub count: u8,
    pub uid: i32,
}

impl LoomFixture {
    pub async fn new(banner: ItemStack, dye_count: u8, pattern: Option<&'static Item>) -> Self {
        let mut fixture = Self::empty().await;
        fixture.bedrock.take_packets();
        fixture.move_input(9, ContainerName::LoomInput, banner);
        fixture.move_input(
            10,
            ContainerName::LoomDye,
            ItemStack::new(dye_count, &Item::RED_DYE),
        );
        if let Some(pattern) = pattern {
            fixture.move_input(11, ContainerName::LoomMaterial, ItemStack::new(1, pattern));
        }
        fixture
    }

    pub async fn empty() -> Self {
        let world = DeathTestWorld::new().await;
        let live_world = world.world();
        publish(&live_world, proto(&Biome::PLAINS, &Block::AIR));
        live_world.set_block_state(
            &POSITION,
            Block::LOOM.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        let bedrock = TestBedrockPlayer::new(&live_world).await;
        bedrock
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 9.5));
        bedrock
            .player
            .screen_handler_sync_handler
            .store_player(bedrock.player.clone());
        open_loom(&bedrock);
        let screen = bedrock
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        assert_eq!(
            screen.lock().unwrap().window_type(),
            Some(pumpkin_data::screen::WindowType::Loom)
        );
        Self {
            world,
            bedrock,
            screen,
            last_packets: Vec::new(),
        }
    }

    fn move_input(&mut self, inventory_slot: u8, name: ContainerName, stack: ItemStack) {
        let count = stack.item_count;
        let expected = stack.clone();
        self.bedrock
            .player
            .inventory()
            .set_stack(inventory_slot as usize, stack);
        let action = Action::Place {
            count,
            source: slot(ContainerName::Inventory, inventory_slot, &expected),
            destination: slot(name, inventory_slot, ItemStack::EMPTY),
        };
        let updates = self.submit(-1, vec![action], 0);
        assert!(
            self.bedrock
                .player
                .inventory()
                .get_stack(inventory_slot as usize)
                .is_empty()
        );
        let actual = self.input(match name {
            ContainerName::LoomInput => 0,
            ContainerName::LoomDye => 1,
            ContainerName::LoomMaterial => 2,
            _ => unreachable!(),
        });
        assert!(
            actual.are_equal(&expected),
            "ordinary input move must succeed"
        );
        assert!(
            updates.iter().any(|update| {
                update.name == name
                    && update.slot == inventory_slot
                    && update.uid == actual.uid.get()
                    && update.count == count
            }),
            "ordinary input move must publish its new stack identity"
        );
    }

    pub fn input(&self, index: usize) -> ItemStack {
        self.screen.lock().unwrap().get_behaviour().slots[index].get_cloned_stack()
    }

    pub fn preview(&self) -> ItemStack {
        self.input(3)
    }

    pub fn selection(&self) -> (i32, Vec<String>) {
        let handler = self.screen.lock().unwrap();
        let loom = handler
            .as_any()
            .downcast_ref::<LoomScreenHandler>()
            .unwrap();
        (
            loom.selected_banner_pattern_index
                .load(std::sync::atomic::Ordering::Relaxed),
            loom.selectable_patterns.clone(),
        )
    }

    pub fn cursor(&self) -> ItemStack {
        self.screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap()
            .clone()
    }

    pub fn craft_actions(&self, pattern: &str, repetitions: u8, cursor: bool) -> Vec<Action> {
        // Source-derived client sequence: Geyser LoomInventoryTranslator and
        // BannerPattern; protocol 2193 CraftResultsDeprecated schema.
        // Dragonfly creates single loom results at CreatedOutput/50.
        vec![
            Action::Loom {
                pattern_id: pattern.into(),
                repetitions,
            },
            Action::CraftResultsDeprecated {
                result_items: vec![self.predicted_banner(pattern)],
                times_crafted: repetitions,
            },
            Action::Consume {
                count: repetitions,
                source: slot(ContainerName::LoomInput, 9, &self.input(0)),
            },
            Action::Consume {
                count: repetitions,
                source: slot(ContainerName::LoomDye, 10, &self.input(1)),
            },
            Action::Take {
                count: repetitions,
                source: RequestSlot {
                    stack_id: -10,
                    ..slot(ContainerName::CreatedOutput, 50, ItemStack::EMPTY)
                },
                destination: if cursor {
                    slot(ContainerName::Cursor, 0, &self.cursor())
                } else {
                    slot(
                        ContainerName::Inventory,
                        12,
                        &self.bedrock.player.inventory().get_stack(12),
                    )
                },
            },
        ]
    }

    fn predicted_banner(&self, pattern: &str) -> StackRequestItem {
        // Geyser BannerItem.convertBannerPattern uses Bedrock IDs and reversed
        // dye IDs in Patterns. Retain the fixture's original layers and name.
        let input = self.input(0);
        let mut layers = input
            .get_data_component::<BannerPatternsImpl>()
            .map_or_else(Vec::new, |patterns| {
                patterns
                    .layers
                    .iter()
                    .map(|layer| {
                        let mut tag = NbtCompound::new();
                        assert_eq!(layer.pattern, "minecraft:stripe_top");
                        tag.put_string("Pattern", "ts".into());
                        tag.put_int("Color", 15 - i32::from(layer.color.id()));
                        NbtTag::Compound(tag)
                    })
                    .collect()
            });
        let mut added = NbtCompound::new();
        added.put_string("Pattern", pattern.into());
        added.put_int("Color", 1); // Bedrock red.
        layers.push(NbtTag::Compound(added));
        let mut tag = NbtCompound::new();
        tag.put_list("Patterns", layers);
        if let Some(name) = input.get_data_component::<CustomNameImpl>() {
            let mut display = NbtCompound::new();
            display.put_string("Name", name.name.clone().get_text());
            tag.put_compound("display", display);
        }
        // ItemInstanceUserData: versioned network NBT, empty placement filters.
        let mut extra_data = Vec::new();
        (-1i16).write(&mut extra_data).unwrap();
        1u8.write(&mut extra_data).unwrap();
        extra_data.extend_from_slice(&Nbt::new(String::new(), tag).write_fixed_le().unwrap());
        0i32.write(&mut extra_data).unwrap();
        0i32.write(&mut extra_data).unwrap();
        StackRequestItem {
            identifier: Some("minecraft:banner".into()),
            metadata_value: VarInt(15),
            count: 1,
            block_runtime_id: VarUInt(0),
            extra_data,
        }
    }

    pub fn submit(&mut self, id: i32, actions: Vec<Action>, expected: u8) -> Vec<ResponseSlot> {
        let (result, updates) = self.dispatch(id, actions);
        assert_eq!(result, expected, "workstation result status");
        updates
    }

    pub fn dispatch(&mut self, id: i32, actions: Vec<Action>) -> (u8, Vec<ResponseSlot>) {
        self.send(id, actions);
        self.response(id)
    }

    pub fn send(&mut self, id: i32, actions: Vec<Action>) {
        self.bedrock.take_packets();
        self.bedrock.client().handle_item_stack_request(
            &self.bedrock.player,
            SItemStackRequest {
                requests: vec![ItemStackRequest {
                    request_id: VarInt(id),
                    actions,
                    filter_strings: Vec::new(),
                    filter_cause: 0,
                }],
            },
        );
        self.last_packets = self.bedrock.take_packets();
    }

    fn response(&self, id: i32) -> (u8, Vec<ResponseSlot>) {
        let (_, status, updates) = responses(&self.last_packets)
            .into_iter()
            .find(|(request, _, _)| *request == id)
            .unwrap();
        (status, updates)
    }

    pub fn assert_no_correction(&self) {
        for packet in &self.last_packets {
            let mut reader = Cursor::new(&packet[1..]);
            VarUInt::read(&mut reader).unwrap();
            let id = VarUInt::read(&mut reader).unwrap().0;
            assert_ne!(
                id,
                CInventoryContent::PACKET_ID as u32,
                "retired session content"
            );
            assert_ne!(id, CInventorySlot::PACKET_ID as u32, "retired session slot");
        }
    }

    pub fn assert_correction(&self) {
        super::super::workstation_correction_test_support::assert_correction(
            &self.last_packets,
            &self.bedrock.player,
        );
    }

    pub fn assert_updates(&self, updates: &[ResponseSlot], cursor: bool) {
        let destination = if cursor {
            (ContainerName::Cursor, 0, self.cursor())
        } else {
            (
                ContainerName::Inventory,
                12,
                self.bedrock.player.inventory().get_stack(12),
            )
        };
        let expected = [
            destination,
            (ContainerName::LoomInput, 9, self.input(0)),
            (ContainerName::LoomDye, 10, self.input(1)),
            (ContainerName::LoomMaterial, 11, self.input(2)),
            (ContainerName::LoomResultPreview, 50, self.preview()),
            (ContainerName::CreatedOutput, 50, ItemStack::EMPTY.clone()),
        ]
        .map(|(name, slot, stack)| ResponseSlot {
            name,
            slot,
            count: stack.item_count,
            uid: if stack.is_empty() { 0 } else { stack.uid.get() },
        });
        assert_eq!(updates, expected, "complete authoritative loom response");
    }

    pub fn snapshot(&self) -> Vec<ItemStack> {
        let mut all: Vec<_> = (0..self.bedrock.player.inventory().size())
            .map(|index| self.bedrock.player.inventory().get_stack(index))
            .collect();
        all.extend((0..4).map(|index| self.input(index)));
        all.push(self.cursor());
        all
    }

    pub fn assert_no_stale_window_after_close(&self) {
        let old_id = self.screen.lock().unwrap().sync_id();
        let mut closed = false;
        let mut acknowledged = false;
        for packet in &self.last_packets {
            let mut reader = Cursor::new(&packet[1..]);
            VarUInt::read(&mut reader).unwrap();
            let header = VarUInt::read(&mut reader).unwrap().0;
            if header == CItemStackResponse::PACKET_ID as u32 {
                assert!(
                    !closed,
                    "committed response followed its reentrant close callback"
                );
                acknowledged = true;
            } else if header == SContainerClose::PACKET_ID as u32 {
                if u8::read(&mut reader).unwrap() == old_id {
                    assert!(
                        acknowledged,
                        "committed response must precede close callbacks"
                    );
                    closed = true;
                }
            } else if closed
                && (header == CInventorySlot::PACKET_ID as u32
                    || header == CInventoryContent::PACKET_ID as u32)
            {
                assert_ne!(
                    VarUInt::read(&mut reader).unwrap().0,
                    u32::from(old_id),
                    "retired loom published after the callback closed it"
                );
            }
        }
        assert!(acknowledged, "committed request must be acknowledged");
        assert!(closed, "callback must close its original loom");
    }

    pub async fn close(self) {
        self.bedrock.close().await;
        self.world.server.shutdown().await;
    }
}

pub(in crate::net::bedrock::play) fn slot(
    name: ContainerName,
    slot_id: u8,
    stack: &ItemStack,
) -> RequestSlot {
    RequestSlot {
        container_name: FullContainerName {
            container_name: name,
            dynamic_id: None,
        },
        slot_id,
        stack_id: if stack.is_empty() { 0 } else { stack.uid.get() },
    }
}

pub(in crate::net::bedrock::play) fn assert_same_stock(
    actual: &[ItemStack],
    expected: &[ItemStack],
    context: impl std::fmt::Debug,
) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            actual.are_equal(expected),
            "{context:?}: stock/components changed at {index}: actual {}x{}, expected {}x{}",
            actual.item_count,
            actual.item.registry_key,
            expected.item_count,
            expected.item.registry_key
        );
        assert_eq!(
            actual.uid, expected.uid,
            "{context:?}: stock identity changed at {index}"
        );
    }
}

pub(in crate::net::bedrock::play) fn open_loom(bedrock: &TestBedrockPlayer) {
    let empty = ItemStack::EMPTY.clone();
    bedrock
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, empty.clone());
    bedrock.client().handle_inventory_action(
        &bedrock.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: true,
            actions: Vec::new(),
            transaction_type: VarUInt(2),
            transaction_data: TransactionData::UseItem(UseItemTransactionData {
                action_type: VarInt(0),
                trigger_type: 0,
                block_position: POSITION,
                block_face: 1,
                hot_bar_slot: VarInt(i32::from(bedrock.player.inventory().get_selected_slot())),
                hand: HandSlot::Mainhand,
                item_in_hand: NetworkItemDescriptor::from(&empty),
                player_position: Vector3::new(8.5, 64.0, 9.5),
                click_position: Vector3::new(0.5, 1.0, 0.5),
                block_runtime_id: VarUInt(0),
                client_prediction: 0,
                client_cooldown_state: 0,
            }),
        },
    );
}

pub(in crate::net::bedrock::play) fn responses(
    packets: &[Bytes],
) -> Vec<(i32, u8, Vec<ResponseSlot>)> {
    let mut responses = Vec::new();
    for packet in packets {
        let mut reader = Cursor::new(&packet[1..]);
        VarUInt::read(&mut reader).unwrap();
        if VarUInt::read(&mut reader).unwrap().0 != CItemStackResponse::PACKET_ID as u32 {
            continue;
        }
        for _ in 0..VarUInt::read(&mut reader).unwrap().0 {
            let result = u8::read(&mut reader).unwrap();
            let id = VarInt::read(&mut reader).unwrap().0;
            let mut decoded = Vec::new();
            let has_containers = bool::read(&mut reader).unwrap();
            if has_containers {
                assert_eq!(result, 0);
                for _ in 0..VarUInt::read(&mut reader).unwrap().0 {
                    let full_name = FullContainerName::read(&mut reader).unwrap();
                    assert_eq!(full_name.dynamic_id, None);
                    for _ in 0..VarUInt::read(&mut reader).unwrap().0 {
                        let requested = u8::read(&mut reader).unwrap();
                        let slot = u8::read(&mut reader).unwrap();
                        assert_eq!(requested, slot);
                        let count = u8::read(&mut reader).unwrap();
                        let uid = if bool::read(&mut reader).unwrap() {
                            VarInt::read(&mut reader).unwrap().0
                        } else {
                            0
                        };
                        assert_eq!(String::read(&mut reader).unwrap(), "");
                        assert_eq!(String::read(&mut reader).unwrap(), "");
                        assert_eq!(VarInt::read(&mut reader).unwrap().0, 0);
                        decoded.push(ResponseSlot {
                            name: full_name.container_name,
                            slot,
                            count,
                            uid,
                        });
                    }
                }
            }
            responses.push((id, result, decoded));
        }
        assert_eq!(reader.position() as usize, packet.len() - 1);
    }
    assert!(
        !responses.is_empty(),
        "workstation request omitted response"
    );
    responses
}
