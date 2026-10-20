#![expect(clippy::unwrap_used, reason = "Bounded real stack-request fixtures")]
use super::{
    created_output_tests::{set_logs, slot},
    rejected_prediction_test_support::{
        Gate, GatedSlot, assert_same, assert_stack, content, helmet, menu_slot, replace_helmet,
        replay_prediction, request, status, take,
    },
};
use crate::{
    entity::death_test_world::DeathTestWorld, net::bedrock::combat_test_support::TestBedrockPlayer,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory, SimpleInventory,
    crafting::crafting_screen_handler::CraftingTableScreenHandler,
    generic_container_screen_handler::create_generic_9x3,
    screen_handler::{ScreenHandler, ScreenMutex},
    slot::Slot,
};
use pumpkin_protocol::{
    bedrock::{
        network_item::{ContainerName, NetworkItemStackDescriptor},
        server::item_stack_request::ItemStackRequestAction as Action,
    },
    codec::var_uint::VarUInt,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    },
    time::Duration,
};

fn craft() -> Vec<Action> {
    vec![
        Action::CraftRecipe {
            recipe_id: VarUInt(1),
            repetitions: 1,
        },
        Action::Take {
            count: 4,
            source: slot(ContainerName::CreatedOutput, 0),
            destination: slot(ContainerName::Inventory, 9),
        },
        Action::Consume {
            count: 1,
            source: slot(ContainerName::CraftingInput, 0),
        },
    ]
}

fn assert_grid(fixture: &TestBedrockPlayer, packets: &[bytes::Bytes], table: bool) {
    let menu = fixture
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    let handler = menu.lock().unwrap();
    let expected_slots = if table { 10 } else { 5 };
    assert_eq!(
        fixture.player.inventory().get_stack(9).item,
        &Item::OAK_PLANKS
    );
    assert_eq!(fixture.player.inventory().get_stack(9).item_count, 4);
    for index in (0..36).filter(|index| *index != 9) {
        assert!(fixture.player.inventory().get_stack(index).is_empty());
    }
    for index in 0..expected_slots {
        let stack = handler.get_behaviour().slots[index].get_cloned_stack();
        if index == 0 {
            assert_eq!((stack.item, stack.item_count), (&Item::OAK_PLANKS, 4));
        } else if index == 1 {
            assert_eq!((stack.item, stack.item_count), (&Item::OAK_LOG, 1));
        } else {
            assert!(stack.is_empty());
        }
    }
    for index in 0..expected_slots {
        let stack = handler.get_behaviour().slots[index].get_cloned_stack();
        let name = if index == 0 {
            ContainerName::CreatedOutput
        } else {
            ContainerName::CraftingInput
        };
        let slot = if index == 0 {
            0
        } else {
            (if table { 32 } else { 28 }) + index as u32 - 1
        };
        let actual = menu_slot(packets, u32::from(handler.sync_id()), name, slot)
            .expect("rejected partial request must correct every active crafting input/output");
        assert_stack(&actual, &stack);
        let predicted = replay_prediction(
            packets,
            u32::from(handler.sync_id()),
            name,
            slot,
            NetworkItemStackDescriptor::default(),
        );
        assert_stack(&predicted, &stack);
    }
}

fn prepare_equipment(fixture: &TestBedrockPlayer) -> ([(usize, ItemStack); 4], ItemStack) {
    let equipment = [
        (36, &Item::DIAMOND_BOOTS),
        (37, &Item::DIAMOND_LEGGINGS),
        (38, &Item::DIAMOND_CHESTPLATE),
        (40, &Item::SHIELD),
    ]
    .map(|(index, item)| {
        let stack = ItemStack::new(1, item);
        fixture.player.inventory().set_stack(index, stack.clone());
        (index, stack)
    });
    let mut cursor = ItemStack::new(1, &Item::DIAMOND_SWORD);
    cursor.add_enchantment(&pumpkin_data::Enchantment::UNBREAKING, 2);
    *fixture
        .player
        .player_screen_handler
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap() = cursor.clone();
    (equipment, cursor)
}

fn assert_equipment_unchanged(
    fixture: &TestBedrockPlayer,
    equipment: &[(usize, ItemStack)],
    cursor: &ItemStack,
) {
    for (index, expected) in equipment {
        assert_same(&fixture.player.inventory().get_stack(*index), expected);
    }
    assert_same(
        &fixture
            .player
            .player_screen_handler
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap(),
        cursor,
    );
}

fn assert_equipment_corrections(
    fixture: &TestBedrockPlayer,
    packets: &[bytes::Bytes],
    equipment: &[(usize, ItemStack)],
    cursor: &ItemStack,
) {
    let (name, corrected) = content(packets, 120).unwrap();
    assert_eq!(name.container_name, ContainerName::Armor);
    assert_eq!(corrected.len(), 4);
    for (stack, index) in corrected.iter().zip((36..40).rev()) {
        assert_stack(stack, &fixture.player.inventory().get_stack(index));
    }
    let (name, corrected) = content(packets, 119).unwrap();
    assert_eq!(name.container_name, ContainerName::Offhand);
    assert_eq!(corrected.len(), 1);
    assert_stack(&corrected[0], &equipment[3].1);
    let (name, corrected) = content(packets, 59).unwrap();
    assert_eq!(name.container_name, ContainerName::Cursor);
    assert_eq!(corrected.len(), 1);
    assert_stack(&corrected[0], cursor);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_partial_stack_request_rejection_corrects_player_crafting_grid() {
    super::rejected_prediction_test_support::bounded(async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        fixture.player.inventory().set_stack(39, helmet(false));
        let (equipment, cursor) = prepare_equipment(&fixture);
        set_logs(&fixture, 2);
        let (gate, entered, resume) = Gate::new();
        let mut handler = fixture.player.player_screen_handler.lock().unwrap();
        let inner = handler.get_behaviour().slots[5].clone();
        handler.get_behaviour_mut().slots[5] = Arc::new(GatedSlot {
            inner,
            gate,
            after_preview: true,
            armed: AtomicBool::new(false),
            committed_destination: Some((fixture.player.inventory().clone(), 9)),
        });
        drop(handler);
        fixture.take_packets();
        let player = fixture.player.clone();
        let mut actions = craft();
        actions.push(take(ContainerName::Armor, 0, ContainerName::Inventory, 10));
        let processing = tokio::task::spawn_blocking(move || {
            let client = player.client.bedrock().unwrap();
            client.handle_item_stack_request(
                &player,
                pumpkin_protocol::bedrock::server::item_stack_request::SItemStackRequest {
                    requests: vec![
                        pumpkin_protocol::bedrock::server::item_stack_request::ItemStackRequest {
                            request_id: pumpkin_protocol::VarInt(1),
                            actions,
                            filter_strings: Vec::new(),
                            filter_cause: 0,
                        },
                    ],
                },
            );
        });
        tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(10)).unwrap())
            .await
            .unwrap();
        // The first craft is committed; the bounded gate precedes the live armor observation.
        assert_eq!(fixture.player.inventory().get_stack(9).item_count, 4);
        let armor = replace_helmet(&world.server, &fixture.player, true);
        resume.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(15), processing)
            .await
            .unwrap()
            .unwrap();
        let packets = fixture.take_packets();
        status(&packets, 1);
        assert_same(&fixture.player.inventory().get_stack(39), &armor);
        assert!(world.world().entities.load().is_empty());
        assert_equipment_unchanged(&fixture, &equipment, &cursor);
        assert_grid(&fixture, &packets, false);
        assert_equipment_corrections(&fixture, &packets, &equipment, &cursor);
        let (name, main) = content(&packets, 0).unwrap();
        assert_eq!(name.container_name, ContainerName::Inventory);
        assert_stack(&main[9], &fixture.player.inventory().get_stack(9));
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_partial_rejection_corrects_crafting_table_control() {
    super::rejected_prediction_test_support::bounded(async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let handler = CraftingTableScreenHandler::new(7, fixture.player.inventory(), None);
        *fixture.player.current_screen_handler.lock().unwrap() =
            Arc::new(ScreenMutex::new(handler));
        set_logs(&fixture, 2);
        fixture.take_packets();
        let mut actions = craft();
        actions.push(take(
            ContainerName::Inventory,
            0,
            ContainerName::Inventory,
            10,
        ));
        request(&fixture, actions);
        let packets = fixture.take_packets();
        status(&packets, 1);
        assert_grid(&fixture, &packets, true);
        assert!(world.world().entities.load().is_empty());
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_partial_rejection_corrects_generic_and_inventory_control() {
    super::rejected_prediction_test_support::bounded(async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let chest: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(27));
        let diamonds = ItemStack::new(2, &Item::DIAMOND);
        chest.set_stack(0, diamonds.clone());
        let handler = create_generic_9x3(
            7,
            fixture.player.inventory(),
            chest.clone(),
            fixture.player.as_ref(),
        );
        *fixture.player.current_screen_handler.lock().unwrap() =
            Arc::new(ScreenMutex::new(handler));
        fixture.take_packets();
        request(
            &fixture,
            vec![
                take(ContainerName::LevelEntity, 0, ContainerName::Inventory, 0),
                take(ContainerName::Inventory, 0, ContainerName::Inventory, 1),
                take(ContainerName::Inventory, 2, ContainerName::Inventory, 3),
            ],
        );
        let packets = fixture.take_packets();
        status(&packets, 1);
        assert_eq!(chest.get_stack(0).item_count, 1);
        assert_eq!(fixture.player.inventory().get_stack(1).item_count, 1);
        assert!(fixture.player.inventory().get_stack(0).is_empty());
        let (name, stacks) = content(&packets, 7).unwrap();
        assert_eq!(name.container_name, ContainerName::LevelEntity);
        assert_eq!(stacks.len(), 27);
        assert_stack(&stacks[0], &chest.get_stack(0));
        let (name, stacks) = content(&packets, 0).unwrap();
        assert_eq!(name.container_name, ContainerName::Inventory);
        for (index, stack) in stacks.iter().enumerate() {
            assert_stack(stack, &fixture.player.inventory().get_stack(index));
        }
        assert!(world.world().entities.load().is_empty());
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await;
}

struct DeferredSlot {
    inner: Arc<dyn Slot>,
    reads: AtomicUsize,
    callback:
        pumpkin_util::sync::Mutex<Option<Box<dyn FnOnce() + Send>>, pumpkin_util::sync::Cursor>,
}
impl Slot for DeferredSlot {
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
    fn get_stack(&self) -> ItemStack {
        let stack = self.inner.get_stack();
        // First read is take-rule preflight; the next is the forced output snapshot.
        if self.reads.fetch_add(1, SeqCst) == 1 {
            crate::world::guard_effects::defer(self.callback.lock().unwrap().take().unwrap())
                .ok()
                .unwrap();
        }
        stack
    }
}

async fn callback_case(kind: u8) {
    let world = DeathTestWorld::new().await;
    let mut fixture = TestBedrockPlayer::new(&world.world()).await;
    fixture.player.inventory().set_stack(39, helmet(true));
    let player = fixture.player.clone();
    let replacement = ItemStack::new(5, &Item::DIAMOND);
    let expected = replacement.clone();
    let callback = move || {
        pumpkin_util::sync::assert_no_guards_held("rejected prediction callback");
        let menu = player.current_screen_handler.lock().unwrap().clone();
        assert!(menu.try_lock().is_ok());
        if kind == 1 {
            let menu = CraftingTableScreenHandler::new(8, player.inventory(), None);
            *menu.get_behaviour().cursor_stack.lock().unwrap() = replacement;
            *player.current_screen_handler.lock().unwrap() = Arc::new(ScreenMutex::new(menu));
        } else if kind == 2 {
            menu.lock().unwrap().get_behaviour().slots[0]
                .get_inventory()
                .set_stack(0, replacement);
        } else {
            *menu
                .lock()
                .unwrap()
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap() = replacement;
        }
    };
    let mut handler = fixture.player.player_screen_handler.lock().unwrap();
    let inner = handler.get_behaviour().slots[0].clone();
    handler.get_behaviour_mut().slots[0] = Arc::new(DeferredSlot {
        inner,
        reads: AtomicUsize::new(0),
        callback: pumpkin_util::sync::Mutex::new(Some(Box::new(callback))),
    });
    drop(handler);
    fixture.take_packets();
    request(
        &fixture,
        vec![take(ContainerName::Armor, 0, ContainerName::Inventory, 9)],
    );
    let packets = fixture.take_packets();
    status(&packets, 1);
    if kind == 1 {
        assert!(
            menu_slot(&packets, 0, ContainerName::CreatedOutput, 0).is_none(),
            "retired menu snapshot must not publish"
        );
        assert!(
            content(&packets, 59).is_none(),
            "old cursor snapshot must not publish after menu replacement"
        );
    } else if kind == 2 {
        let actual = menu_slot(&packets, 0, ContainerName::CreatedOutput, 0)
            .expect("same-target callback must publish live output");
        assert_stack(&actual, &expected);
    } else {
        let (name, stacks) = content(&packets, 59).unwrap();
        assert_eq!(name.container_name, ContainerName::Cursor);
        assert_stack(&stacks[0], &expected);
    }
    let menu = fixture
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    if kind == 2 {
        assert_same(
            &menu.lock().unwrap().get_behaviour().slots[0]
                .get_inventory()
                .get_stack(0),
            &expected,
        );
    } else {
        assert_same(
            &menu
                .lock()
                .unwrap()
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap(),
            &expected,
        );
    }
    fixture.close().await;
    world.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_menu_replacement_prevents_stale_correction() {
    super::rejected_prediction_test_support::bounded(async {
        callback_case(1).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_callback_cursor_replacement_is_published_live() {
    super::rejected_prediction_test_support::bounded(async {
        callback_case(0).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_same_target_callback_revalidates_output() {
    super::rejected_prediction_test_support::bounded(async {
        callback_case(2).await;
    })
    .await;
}
