#![expect(
    clippy::unwrap_used,
    reason = "Real packet fixtures require successful setup"
)]

use super::{created_output_tests::slot, inventory_exploit_tests::packet_payload};
use crate::{entity::player::Player, net::bedrock::combat_test_support::TestBedrockPlayer};
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{Inventory, screen_handler::InventoryPlayer, slot::Slot};
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
        server::item_stack_request::{
            ItemStackRequest, ItemStackRequestAction as Action, SItemStackRequest,
        },
    },
    codec::var_uint::VarUInt,
    serial::PacketRead,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::SeqCst},
    mpsc,
};
use std::time::Duration;

pub(super) fn helmet(cursed: bool) -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::DIAMOND_HELMET);
    if cursed {
        stack.add_enchantment(&Enchantment::BINDING_CURSE, 1);
    }
    stack
}

pub(super) fn take(source: ContainerName, from: u8, destination: ContainerName, to: u8) -> Action {
    Action::Take {
        count: 1,
        source: slot(source, from),
        destination: slot(destination, to),
    }
}

pub(super) fn request(fixture: &TestBedrockPlayer, actions: Vec<Action>) {
    fixture.client().handle_item_stack_request(
        &fixture.player,
        SItemStackRequest {
            requests: vec![ItemStackRequest {
                request_id: VarInt(1),
                actions,
                filter_strings: Vec::new(),
                filter_cause: 0,
            }],
        },
    );
}

pub(super) fn status(packets: &[bytes::Bytes], expected: u8) {
    let mut bytes = packets
        .iter()
        .find_map(|packet| packet_payload(packet, 148))
        .unwrap();
    assert_eq!(VarUInt::read(&mut bytes).unwrap().0, 1);
    assert_eq!(
        u8::read(&mut bytes).unwrap(),
        expected,
        "request response status"
    );
    assert_eq!(VarInt::read(&mut bytes).unwrap().0, 1);
    if expected != 0 {
        assert!(
            !bool::read(&mut bytes).unwrap(),
            "error cannot advertise successful slots"
        );
        assert!(bytes.is_empty());
    }
}

pub(super) fn content(
    packets: &[bytes::Bytes],
    window: u32,
) -> Option<(FullContainerName, Vec<NetworkItemStackDescriptor>)> {
    let mut bytes = packets.iter().rev().find_map(|packet| {
        let mut bytes = packet_payload(packet, 49)?;
        (VarUInt::read(&mut bytes).unwrap().0 == window).then_some(bytes)
    })?;
    let count = VarUInt::read(&mut bytes).unwrap().0;
    let stacks = (0..count)
        .map(|_| NetworkItemStackDescriptor::read(&mut bytes).unwrap())
        .collect();
    let name = FullContainerName::read(&mut bytes).unwrap();
    let storage = NetworkItemStackDescriptor::read(&mut bytes).unwrap();
    assert_eq!(storage.id, 0);
    assert!(bytes.is_empty());
    Some((name, stacks))
}

pub(super) fn menu_slot(
    packets: &[bytes::Bytes],
    window: u32,
    name: ContainerName,
    slot: u32,
) -> Option<NetworkItemStackDescriptor> {
    packets.iter().rev().find_map(|packet| {
        let mut bytes = packet_payload(packet, 50)?;
        let actual_window = VarUInt::read(&mut bytes).unwrap().0;
        let actual_slot = VarUInt::read(&mut bytes).unwrap().0;
        let actual_name = Option::<FullContainerName>::read(&mut bytes).unwrap()?;
        let storage = Option::<NetworkItemStackDescriptor>::read(&mut bytes).unwrap();
        let stack = NetworkItemStackDescriptor::read(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert!(storage.is_none());
        (actual_window == window
            && actual_slot == slot
            && actual_name.container_name == name
            && actual_name.dynamic_id.is_none())
        .then_some(stack)
    })
}

pub(super) fn assert_stack(actual: &NetworkItemStackDescriptor, expected: &ItemStack) {
    let expected = NetworkItemStackDescriptor::from(expected);
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.stack_size, expected.stack_size);
    assert_eq!(actual.aux_value.0, expected.aux_value.0);
    assert_eq!(actual.net_id, expected.net_id);
    assert_eq!(actual.extra_data, expected.extra_data);
}

pub(super) fn assert_same(actual: &ItemStack, expected: &ItemStack) {
    assert_eq!(actual.uid, expected.uid);
    assert!(actual.are_equal(expected));
}

pub(super) struct Gate {
    entered: mpsc::Sender<()>,
    resume: pumpkin_util::sync::Mutex<mpsc::Receiver<()>, pumpkin_util::sync::Cursor>,
    once: AtomicBool,
}

impl Gate {
    pub(super) fn new() -> (Arc<Self>, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, receive) = mpsc::channel();
        let (send, resume) = mpsc::channel();
        (
            Arc::new(Self {
                entered,
                resume: pumpkin_util::sync::Mutex::new(resume),
                once: AtomicBool::new(true),
            }),
            receive,
            send,
        )
    }
    fn pause(&self) {
        if self.once.swap(false, SeqCst) {
            self.entered.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    }
}

pub(super) struct GatedSlot {
    pub(super) inner: Arc<dyn Slot>,
    pub(super) gate: Arc<Gate>,
    pub(super) after_preview: bool,
    pub(super) armed: AtomicBool,
    pub(super) committed_destination: Option<(Arc<dyn Inventory>, usize)>,
}

impl Slot for GatedSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.inner.get_inventory()
    }
    fn get_index(&self) -> usize {
        self.inner.get_index()
    }
    fn set_id(&self, id: usize) {
        self.inner.set_id(id);
    }
    fn mark_dirty(&self) {
        self.inner.mark_dirty();
    }
    fn can_insert(&self, stack: &ItemStack) -> bool {
        self.inner.can_insert(stack)
    }
    fn get_max_item_count(&self) -> u8 {
        self.inner.get_max_item_count()
    }
    fn get_stack(&self) -> ItemStack {
        // CraftRecipe refreshes slots before payment; pause only after the earlier commit.
        let committed = self
            .committed_destination
            .as_ref()
            .is_none_or(|(inventory, index)| !inventory.get_stack(*index).is_empty());
        if self.after_preview && committed && self.armed.swap(false, SeqCst) {
            self.gate.pause();
        }
        self.inner.get_stack()
    }
    fn can_take_items(&self, player: &dyn InventoryPlayer) -> bool {
        let allowed = self.inner.can_take_items(player);
        if allowed && !self.after_preview {
            self.gate.pause();
        }
        allowed
    }
    fn can_take_stack(&self, player: &dyn InventoryPlayer, stack: &ItemStack) -> bool {
        let allowed = self.inner.can_take_stack(player, stack);
        if allowed && self.after_preview {
            self.armed.store(true, SeqCst);
        }
        allowed
    }
    fn on_take_item(&self, player: &dyn InventoryPlayer, stack: &ItemStack) {
        self.inner.on_take_item(player, stack);
    }
    fn set_stack_prev(&self, stack: ItemStack, previous: ItemStack) {
        self.inner.set_stack_prev(stack, previous);
    }
}

pub(super) fn replace_helmet(
    server: &Arc<crate::server::Server>,
    player: &Player,
    cursed: bool,
) -> ItemStack {
    let source = crate::command::CommandSender::Console.into_source(server);
    let command = if cursed {
        "item replace entity @a armor.head with minecraft:diamond_helmet[enchantments={\"minecraft:binding_curse\":1}]"
    } else {
        "item replace entity @a armor.head with minecraft:diamond_helmet"
    };
    assert_eq!(
        server
            .command_dispatcher
            .load()
            .execute_input(command, &source)
            .unwrap(),
        1
    );
    let installed = player.inventory().get_stack(39);
    assert_eq!(
        installed.get_enchantment_level(&Enchantment::BINDING_CURSE),
        i32::from(cursed)
    );
    installed
}

pub(super) async fn bounded(operation: impl std::future::Future<Output = ()>) {
    tokio::time::timeout(Duration::from_secs(120), operation)
        .await
        .unwrap();
}

// Apply the server's wire corrections in order to a client's already predicted slot.
pub(super) fn replay_prediction(
    packets: &[bytes::Bytes],
    window: u32,
    name: ContainerName,
    index: u32,
    mut predicted: NetworkItemStackDescriptor,
) -> NetworkItemStackDescriptor {
    for packet in packets {
        let one = std::slice::from_ref(packet);
        if let Some((address, stacks)) = content(one, window)
            && address.container_name == name
            && let Some(stack) = stacks.get(index as usize)
        {
            predicted = stack.clone();
        }
        if let Some(stack) = menu_slot(one, window, name, index) {
            predicted = stack;
        }
    }
    predicted
}
