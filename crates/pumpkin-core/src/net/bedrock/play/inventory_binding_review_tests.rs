#![expect(
    clippy::unwrap_used,
    reason = "Packet regression fixtures must construct successfully"
)]

use super::{
    created_output_tests::{set_logs, slot},
    inventory_exploit_tests::packet_payload,
};
use crate::{
    entity::{death_test_world::DeathTestWorld, player::Player},
    net::bedrock::combat_test_support::TestBedrockPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::Server,
};
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{
    Inventory, SimpleInventory,
    generic_container_screen_handler::create_generic_9x3,
    screen_handler::{ScreenHandler, ScreenMutex},
};
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
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

fn cursed_helmet() -> ItemStack {
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    helmet.add_enchantment(&Enchantment::BINDING_CURSE, 1);
    helmet
}

fn take(source: ContainerName, from: u8, destination: ContainerName, to: u8) -> Action {
    Action::Take {
        count: 1,
        source: slot(source, from),
        destination: slot(destination, to),
    }
}

fn drop_armor() -> Action {
    Action::Drop {
        count: 1,
        source: slot(ContainerName::Armor, 0),
        randomly: false,
    }
}

fn request(fixture: &mut TestBedrockPlayer, actions: Vec<Action>) -> Vec<bytes::Bytes> {
    fixture.take_packets();
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
    let packets = fixture.take_packets();
    let mut response = packets
        .iter()
        .find_map(|packet| packet_payload(packet, 148))
        .unwrap();
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    assert_eq!(u8::read(&mut response).unwrap(), 1);
    assert_eq!(VarInt::read(&mut response).unwrap().0, 1);
    assert!(!bool::read(&mut response).unwrap());
    assert!(
        response.is_empty(),
        "a rejected request reported successful slots"
    );
    packets
}

fn all_slots(player: &Player) -> Vec<ItemStack> {
    let current = player.current_screen_handler.lock().unwrap().clone();
    let mut stacks = {
        let handler = current.lock().unwrap();
        let mut stacks: Vec<_> = handler
            .get_behaviour()
            .slots
            .iter()
            .map(|slot| slot.get_cloned_stack())
            .collect();
        stacks.push(handler.get_behaviour().cursor_stack.lock().unwrap().clone());
        stacks
    };
    let handler = player.player_screen_handler.lock().unwrap();
    stacks.extend(
        handler
            .get_behaviour()
            .slots
            .iter()
            .map(|slot| slot.get_cloned_stack()),
    );
    stacks.push(handler.get_behaviour().cursor_stack.lock().unwrap().clone());
    stacks
}

fn assert_unchanged(before: &[ItemStack], player: &Player) {
    let after = all_slots(player);
    assert_eq!(before.len(), after.len());
    for (index, (before, after)) in before.iter().zip(&after).enumerate() {
        assert!(
            before.are_equal(after),
            "rejected request changed slot {index}"
        );
        assert_eq!(
            before.uid, after.uid,
            "rejected request replaced slot {index}"
        );
    }
}

struct CountStatistics(Arc<AtomicUsize>);
impl EventHandler<PlayerStatisticIncrementEvent> for CountStatistics {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

async fn rejected_before_effects(kind: usize) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let calls = Arc::new(AtomicUsize::new(0));
        world
            .server
            .plugin_manager
            .register::<PlayerStatisticIncrementEvent, _>(
                Arc::new(CountStatistics(calls.clone())),
                EventPriority::Normal,
                true,
            );
        fixture.player.inventory().set_stack(39, cursed_helmet());
        fixture
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::DIAMOND));
        set_logs(&fixture, 2);
        let mut actions = [
            vec![take(
                ContainerName::Inventory,
                0,
                ContainerName::Inventory,
                1,
            )],
            vec![Action::Drop {
                count: 1,
                source: slot(ContainerName::Inventory, 0),
                randomly: false,
            }],
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
            ],
        ]
        .into_iter()
        .nth(kind)
        .unwrap();
        actions.push(drop_armor());
        let before = all_slots(&fixture.player);
        let statistics = fixture.player.stats.lock().unwrap().stats.clone();
        let entities = world.world().entities.load().len();
        request(&mut fixture, actions);
        assert_eq!(
            world.world().entities.load().len(),
            entities,
            "rejected request dropped an entity"
        );
        assert_eq!(fixture.player.stats.lock().unwrap().stats, statistics);
        assert_eq!(
            calls.load(Relaxed),
            0,
            "rejected request published crafting effects"
        );
        assert_unchanged(&before, &fixture.player);
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_rejects_before_slots_change() {
    rejected_before_effects(0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_rejects_before_drop_effects() {
    rejected_before_effects(1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_rejects_before_crafting_effects() {
    rejected_before_effects(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_checks_armor_equipped_by_an_earlier_action() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        fixture.player.inventory().set_stack(0, cursed_helmet());
        let before = all_slots(&fixture.player);
        request(
            &mut fixture,
            vec![
                take(ContainerName::Inventory, 0, ContainerName::Armor, 0),
                drop_armor(),
            ],
        );
        assert_unchanged(&before, &fixture.player);
        assert!(world.world().entities.load().is_empty());
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_checks_repaired_armor_from_staged_inputs() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        for index in 0..2 {
            let mut helmet = cursed_helmet();
            helmet.set_damage(100);
            fixture.player.inventory().set_stack(index, helmet);
        }
        let before = all_slots(&fixture.player);
        request(
            &mut fixture,
            vec![
                take(ContainerName::Inventory, 0, ContainerName::CraftingInput, 0),
                take(ContainerName::Inventory, 1, ContainerName::CraftingInput, 1),
                Action::CraftRecipe {
                    recipe_id: VarUInt(1),
                    repetitions: 1,
                },
                take(ContainerName::CreatedOutput, 0, ContainerName::Armor, 0),
                drop_armor(),
            ],
        );
        assert_unchanged(&before, &fixture.player);
        assert!(world.world().entities.load().is_empty());
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}

async fn rejects_grid_reuse(pending: bool) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let mut helmet = cursed_helmet();
        helmet.set_damage(100);
        if pending {
            let handler = fixture.player.player_screen_handler.lock().unwrap();
            for index in 1..=2 {
                handler.get_slot(index).set_stack(helmet.copy_with_count(1));
            }
            handler
                .get_slot(0)
                .on_quick_move_crafted(ItemStack::EMPTY.clone(), ItemStack::EMPTY.clone());
        } else {
            fixture.player.inventory().set_stack(0, helmet);
            set_logs(&fixture, 1);
        }
        let mut actions = vec![Action::CraftRecipe {
            recipe_id: VarUInt(1),
            repetitions: 1,
        }];
        if pending {
            actions.push(Action::Consume {
                count: 1,
                source: slot(ContainerName::CraftingInput, 0),
            });
        } else {
            actions.extend([
                take(ContainerName::CreatedOutput, 0, ContainerName::Inventory, 9),
                take(ContainerName::Inventory, 0, ContainerName::CraftingInput, 0),
            ]);
        }
        actions.extend([
            take(ContainerName::CraftingInput, 0, ContainerName::Armor, 0),
            drop_armor(),
        ]);
        let before = all_slots(&fixture.player);
        request(&mut fixture, actions);
        assert_unchanged(&before, &fixture.player);
        assert!(world.world().entities.load().is_empty());
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_preserves_pending_crafting_inputs() {
    rejects_grid_reuse(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_tracks_paid_crafting_inputs() {
    rejects_grid_reuse(false).await;
}

fn content(
    packets: &[bytes::Bytes],
    window: u32,
) -> (Vec<NetworkItemStackDescriptor>, FullContainerName) {
    let mut bytes = packets
        .iter()
        .find_map(|packet| {
            let mut bytes = packet_payload(packet, 49)?;
            (VarUInt::read(&mut bytes).unwrap().0 == window).then_some(bytes)
        })
        .unwrap();
    let count = VarUInt::read(&mut bytes).unwrap().0;
    let slots = (0..count)
        .map(|_| NetworkItemStackDescriptor::read(&mut bytes).unwrap())
        .collect();
    (slots, FullContainerName::read(&mut bytes).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_corrects_rejected_open_chest_prediction() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let chest: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(27));
        chest.set_stack(0, ItemStack::new(3, &Item::DIAMOND));
        fixture
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::STICK));
        let mut handler = create_generic_9x3(
            7,
            fixture.player.inventory(),
            chest.clone(),
            fixture.player.as_ref(),
        );
        handler.send_content_updates();
        *fixture.player.current_screen_handler.lock().unwrap() =
            Arc::new(ScreenMutex::new(handler));
        let before = all_slots(&fixture.player);
        let packets = request(
            &mut fixture,
            vec![take(
                ContainerName::LevelEntity,
                0,
                ContainerName::Inventory,
                0,
            )],
        );
        assert_unchanged(&before, &fixture.player);
        let (slots, name) = content(&packets, 7);
        assert_eq!(name.container_name, ContainerName::LevelEntity);
        assert_eq!(slots.len(), 27);
        for (index, actual) in slots.iter().enumerate() {
            let expected = NetworkItemStackDescriptor::from(&chest.get_stack(index));
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.stack_size, expected.stack_size);
            assert_eq!(actual.net_id, expected.net_id);
        }
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_binding_review_invalid_shapes_do_not_amplify_snapshots() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        fixture.player.inventory().set_stack(39, cursed_helmet());
        let before = all_slots(&fixture.player);
        for _ in 0..8 {
            for action in [
                take(ContainerName::Inventory, 255, ContainerName::Inventory, 0),
                take(
                    ContainerName::CraftingOutputPreview,
                    0,
                    ContainerName::Inventory,
                    0,
                ),
                take(ContainerName::Inventory, 0, ContainerName::HotBar, 0),
            ] {
                let packets = request(&mut fixture, vec![action]);
                assert!(
                    !packets
                        .iter()
                        .any(|packet| packet_payload(packet, 49).is_some()),
                    "malformed request triggered full inventories"
                );
            }
        }
        let packets = request(
            &mut fixture,
            vec![take(ContainerName::Armor, 0, ContainerName::Inventory, 0)],
        );
        let (armor, _) = content(&packets, 120);
        assert_eq!(armor.len(), 4);
        assert_eq!(
            armor[0].id,
            NetworkItemStackDescriptor::from(&cursed_helmet()).id
        );
        assert_unchanged(&before, &fixture.player);
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await
    .unwrap();
}
