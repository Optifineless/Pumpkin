use super::callback::{CancelPlacement, SelectSlot, assert_observer_hand, thrown_eggs};
use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{
        player::player_statistic_increment::PlayerStatisticIncrementEvent,
        server::packet::PacketSentEvent,
    },
    block::block_place::BlockPlaceEvent,
};
use crate::server::Server;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_inventory::Inventory;
use pumpkin_protocol::MultiVersionJavaPacket;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

fn conserved_items(fixture: &HandUseFixture) -> usize {
    let player = &fixture.owner.player;
    let inventory = player.inventory();
    let count: usize = (0..inventory.size())
        .map(|slot| usize::from(inventory.get_stack(slot).item_count))
        .sum();
    let cursor = player
        .player_screen_handler
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .clone();
    let cursor_count = usize::from(cursor.lock().unwrap().item_count);
    count + cursor_count + thrown_eggs(&fixture.world.world())
}

pub(in super::super) async fn unrecognised_prediction_placement(denied: bool) -> TestResult {
    for hand in [Hand::Left, Hand::Right] {
        let mut fixture = HandUseFixture::new().await;
        let world = fixture.world.world();
        let (position, before) = prepare_hand_use(&fixture, BlockUse::Placement, hand, 2);
        if denied {
            fixture
                .world
                .server
                .plugin_manager
                .register::<BlockPlaceEvent, _>(
                    Arc::new(CancelPlacement),
                    EventPriority::Normal,
                    true,
                );
        }
        let total = conserved_items(&fixture);
        let (container, slot, _) = hand_address(hand);
        let packet = click_block_packet(
            &fixture.owner.player,
            if hand == Hand::Left {
                HandSlot::Offhand
            } else {
                HandSlot::Mainhand
            },
            position,
            &before,
            vec![action(
                Some(container),
                slot,
                NetworkItemDescriptor::from(&before),
                NetworkItemDescriptor::default(),
            )],
        );
        fixture.drain_packets();
        fixture
            .owner
            .client()
            .handle_inventory_action(&fixture.owner.player, packet);
        fixture
            .verify_and_close(|fixture| {
                let consumed = u8::from(!denied);
                let mut expected = before.clone();
                expected.decrement(consumed);
                let actual = fixture.owner.player.inventory().get_stack_in_hand(hand);
                assert_eq!(
                    actual.item_count, expected.item_count,
                    "2 -> EMPTY is advisory"
                );
                assert!(
                    actual.are_equal(&expected),
                    "authoritative components retained"
                );
                assert_eq!(actual.uid, before.uid);
                assert_eq!(
                    world.get_block(&BlockPos::new(8, 65, 8)),
                    if denied {
                        &Block::AIR
                    } else {
                        &Block::GLOWSTONE
                    }
                );
                assert_eq!(
                    conserved_items(fixture) + usize::from(consumed),
                    total,
                    "only successful placement may consume one item"
                );
                let owner = fixture.owner.take_packets();
                assert_owner_packet(fixture, &owner, hand, &expected)?;
                assert_observer_hand(fixture, hand, &expected)
            })
            .await?;
    }
    Ok(())
}

struct SelectDuringEquipment {
    player: Arc<Player>,
    selected: u8,
    fired: AtomicBool,
}

impl EventHandler<PacketSentEvent> for SelectDuringEquipment {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == CSetEquipment::to_id(CURRENT_MC_VERSION)
            && !self.fired.swap(true, SeqCst)
        {
            self.player.inventory().set_selected_slot(self.selected);
        }
        Box::pin(async {})
    }
}

impl EventHandler<PlayerInteractEvent> for SelectDuringEquipment {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.fired.store(true, SeqCst);
        self.player.inventory().set_selected_slot(self.selected);
        Box::pin(async {})
    }
}

impl EventHandler<PlayerStatisticIncrementEvent> for SelectDuringEquipment {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        self.fired.store(true, SeqCst);
        self.player.inventory().set_selected_slot(self.selected);
        Box::pin(async {})
    }
}

enum SelectionCallback {
    Equipment,
    Statistic,
    Interact,
}

fn select_during_equipment(fixture: &HandUseFixture, selected: u8) -> Arc<SelectDuringEquipment> {
    let java = fixture.java_observer.client();
    java.set_player(fixture.java_observer.player.clone());
    java.version.store(JavaMinecraftVersion::V_26_2);
    let handler = Arc::new(SelectDuringEquipment {
        player: fixture.owner.player.clone(),
        selected,
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(handler.clone(), EventPriority::Normal, true);
    handler
}

pub(super) fn assert_live_main(
    fixture: &mut HandUseFixture,
    expected: &ItemStack,
    selected: u8,
) -> TestResult {
    let entity_id = fixture.owner.player.entity_id();
    let java = CSetEquipment::new(
        entity_id.into(),
        vec![(
            EquipmentSlot::MAIN_HAND.discriminant(),
            ItemStackSerializer::from(expected.clone()),
        )],
    );
    let encoded = fixture.java_observer.client().serialize_packet(&java)?;
    assert!(
        fixture.java_observer.take_packets().contains(&encoded),
        "Java live hand"
    );
    let bedrock = CMobEquipment {
        target_runtime_id: (entity_id as u64).into(),
        item: NetworkItemStackDescriptor::from(expected),
        slot: selected,
        selected_slot: selected,
        container_id: WINDOW_ID_INVENTORY as u8,
    };
    let encoded = fixture
        .bedrock_observer
        .client()
        .serialize_packet(&bedrock)?;
    assert!(
        fixture.bedrock_observer.take_packets().contains(&encoded),
        "Bedrock live hand"
    );
    Ok(())
}

async fn air_selection_callback(phase: SelectionCallback) -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let eggs = ItemStack::new(2, &Item::BROWN_EGG);
    let diamonds = ItemStack::new(3, &Item::DIAMOND);
    player.inventory().set_stack(4, eggs.clone());
    player.inventory().set_stack(8, diamonds.clone());
    // Predicted transfers stay advisory; committed egg statistics follow claim and spawn.
    let mut actions = Vec::new();
    let handler = match phase {
        SelectionCallback::Equipment => {
            actions = callback::pre_use_equipment_actions(&player, Hand::Right);
            select_during_equipment(&fixture, 8)
        }
        SelectionCallback::Statistic | SelectionCallback::Interact => {
            let handler = Arc::new(SelectDuringEquipment {
                player: player.clone(),
                selected: 8,
                fired: AtomicBool::new(false),
            });
            if matches!(phase, SelectionCallback::Statistic) {
                fixture
                    .world
                    .server
                    .plugin_manager
                    .register::<PlayerStatisticIncrementEvent, _>(
                        handler.clone(),
                        EventPriority::Normal,
                        true,
                    );
            } else {
                fixture
                    .world
                    .server
                    .plugin_manager
                    .register::<PlayerInteractEvent, _>(
                        handler.clone(),
                        EventPriority::Normal,
                        true,
                    );
            }
            handler
        }
    };
    actions.insert(
        0,
        action(
            Some(WINDOW_ID_INVENTORY),
            4,
            NetworkItemDescriptor::from(&eggs),
            NetworkItemDescriptor::from(&eggs.copy_with_count(1)),
        ),
    );
    let mut packet = click_block_packet(
        &fixture.owner.player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &eggs,
        actions,
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(1);
    }
    let total = conserved_items(&fixture);
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(handler.fired.load(SeqCst), "real equipment callback");
            assert_eq!(
                thrown_eggs(&fixture.world.world()),
                usize::from(!matches!(phase, SelectionCallback::Interact)),
                "interaction callbacks reject stale use; committed statistics and publication follow use"
            );
            let expected = if matches!(phase, SelectionCallback::Interact) {
                eggs.clone()
            } else {
                let mut remaining = eggs.clone();
                remaining.decrement(1);
                remaining
            };
            assert!(player.inventory().get_stack(4).are_equal(&expected));
            assert!(player.inventory().get_stack(8).are_equal(&diamonds));
            assert_eq!(
                conserved_items(fixture),
                total,
                "selection consumes nothing"
            );
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Right, &expected)?;
            assert_live_main(fixture, &diamonds, 8)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoritative_air_selection_callback_never_debits_diamonds() -> TestResult {
    air_selection_callback(SelectionCallback::Equipment).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoritative_air_statistic_callback_never_debits_diamonds() -> TestResult {
    air_selection_callback(SelectionCallback::Statistic).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoritative_air_interact_callback_never_debits_diamonds() -> TestResult {
    air_selection_callback(SelectionCallback::Interact).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoritative_final_publication_selection_uses_empty_hand_damage() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    let world = fixture.world.world();
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    player.inventory().set_selected_slot(4);
    let blocks = ItemStack::new(1, &Item::STONE);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    player.inventory().set_stack(4, blocks.clone());
    player.inventory().set_stack(8, sword.clone());
    player
        .living_entity
        .send_equipment_changes(&[(EquipmentSlot::MAIN_HAND, blocks.clone())]);
    fixture.owner.player.tick(&fixture.world.server);
    let empty_damage = player
        .living_entity
        .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(
            Arc::new(SelectSlot(calls.clone())),
            EventPriority::Normal,
            true,
        );
    let handler = select_during_equipment(&fixture, 4);
    let total = conserved_items(&fixture);
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(
            &fixture.owner.player,
            HandSlot::Mainhand,
            position,
            &blocks,
            Vec::new(),
        ),
    );
    fixture
        .verify_and_close(|fixture| {
            assert_eq!(calls.load(SeqCst), 1);
            assert!(handler.fired.load(SeqCst));
            assert_eq!(player.inventory().get_selected_slot(), 4);
            assert!(player.inventory().get_stack(4).is_empty());
            assert!(player.inventory().get_stack(8).are_equal(&sword));
            assert_eq!(world.get_block(&BlockPos::new(8, 65, 8)), &Block::STONE);
            assert_eq!(
                conserved_items(fixture) + 1,
                total,
                "only placement consumes"
            );
            player
                .get_entity()
                .on_ground
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let victim = fixture.java_observer.player.clone();
            let health = victim.living_entity.health.load();
            player.attack(&(victim.clone() as Arc<dyn EntityBase>));
            assert_eq!(
                health - victim.living_entity.health.load(),
                empty_damage,
                "real attack must follow the empty live hand"
            );
            let owner = fixture.owner.take_packets();
            assert_owner_packet(fixture, &owner, Hand::Right, ItemStack::EMPTY)?;
            assert_live_main(fixture, ItemStack::EMPTY, 4)?;
            Ok(())
        })
        .await
}

struct CountAirInteractions(Arc<AtomicUsize>);

impl EventHandler<PlayerInteractEvent> for CountAirInteractions {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, SeqCst);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_use_statistic_selection_rejects_before_interaction_dispatch() -> TestResult {
    let mut fixture = HandUseFixture::new().await;
    let player = fixture.owner.player.clone();
    player.inventory().set_selected_slot(4);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    let diamonds = ItemStack::new(3, &Item::DIAMOND);
    player.inventory().set_stack(4, sword.clone());
    player.inventory().set_stack(8, diamonds.clone());
    let selection = Arc::new(SelectDuringEquipment {
        player: player.clone(),
        selected: 8,
        fired: AtomicBool::new(false),
    });
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            selection.clone(),
            EventPriority::Normal,
            true,
        );
    let interactions = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(
            Arc::new(CountAirInteractions(interactions.clone())),
            EventPriority::Normal,
            true,
        );
    let mut packet = click_block_packet(
        &player,
        HandSlot::Mainhand,
        BlockPos::new(8, 64, 8),
        &sword,
        Vec::new(),
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(1);
    }
    fixture.drain_packets();
    fixture
        .owner
        .client()
        .handle_inventory_action(&player, packet);
    fixture
        .verify_and_close(|fixture| {
            assert!(selection.fired.load(SeqCst));
            assert_eq!(
                interactions.load(SeqCst),
                0,
                "stale use must stop before the interaction event"
            );
            assert_eq!(player.inventory().get_selected_slot(), 8);
            assert!(player.inventory().get_stack(4).are_equal(&sword));
            assert!(player.inventory().get_stack(8).are_equal(&diamonds));
            assert_live_main(fixture, &diamonds, 8)
        })
        .await
}
