use crate::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent,
    },
    server::Server,
};
use pumpkin_data::{item::Item, item_stack::ItemStack, statistic::StatisticCategory};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::java::server::play::SlotActionType;
use pumpkin_util::{GameMode, Hand};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

struct CheckClaimBeforeSpawn {
    player: Arc<Player>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<EntitySpawnEvent> for CheckClaimBeforeSpawn {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        let creative = self.player.gamemode.load() == GameMode::Creative;
        let hand = if creative { Hand::Left } else { Hand::Right };
        assert_eq!(
            self.player.inventory.get_stack_in_hand(hand).item_count,
            if creative { 2 } else { 1 },
            "projectile published before its consumption committed"
        );
        let owner = self.player.inventory.slot_transaction(0);
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    assert!(
                        owner.try_lock().is_some(),
                        "spawn callback retained inventory ownership"
                    );
                })
                .join()
                .unwrap();
        });
        self.calls.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

struct CancelSpawn(Arc<AtomicUsize>);
impl EventHandler<EntitySpawnEvent> for CancelSpawn {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::death_test_world::DeathTestWorld,
    inventory_transaction_tests::{click, open_chest},
    net::java::combat_test_support::TestPlayer,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_throw_rejects_hand_swapped_after_dispatch_snapshot() {
    let fixture = DeathTestWorld::new().await;
    let player = TestPlayer::new(&fixture.world());
    // The real dispatcher accepts this observation at its module boundary. A real
    // number-key click deterministically changes the hand before that observation commits.
    for item in [
        &Item::ENDER_PEARL,
        &Item::EGG,
        &Item::EXPERIENCE_BOTTLE,
        &Item::SNOWBALL,
        &Item::SPLASH_POTION,
        &Item::LINGERING_POTION,
        &Item::WIND_CHARGE,
    ] {
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, item));
        let observed = player.player.inventory.get_stack_in_hand(Hand::Right);
        let chest = Arc::new(ChestBlockEntity::new(
            pumpkin_util::math::position::BlockPos::new(8, 64, 8),
        ));
        let replacement = ItemStack::new(1, &Item::STONE);
        chest.set_stack(0, replacement.clone());
        open_chest(&player, chest.clone());
        let mut swap = click();
        swap.mode = SlotActionType::Swap;
        player.player.on_slot_click(swap, &fixture.server);
        let moved = chest.get_stack(0);
        let swapped = player.player.inventory.held_item();
        assert!(swapped.are_equal(&replacement));
        fixture
            .server
            .item_registry
            .on_use(&observed, &player.player, Hand::Right);
        assert!(
            fixture.world().entities.load().is_empty(),
            "stale {item:?} use spawned a projectile"
        );
        let remaining = player.player.inventory.held_item();
        assert_eq!(remaining.uid, swapped.uid);
        assert!(remaining.are_equal(&replacement));
        assert!(chest.get_stack(0).are_equal(&observed));
        assert_eq!(chest.get_stack(0).uid, moved.uid);
        assert_eq!(
            player
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(item.id)),
            0
        );
        assert!(!player.player.has_item_cooldown(item.registry_key));
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_pearl_use_consumes_before_one_projectile() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(
            Arc::new(CheckClaimBeforeSpawn {
                player: player.player.clone(),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    for (hand, mode, count) in [
        (Hand::Right, GameMode::Survival, 1),
        (Hand::Left, GameMode::Creative, 2),
    ] {
        player.player.gamemode.store(mode);
        player
            .player
            .inventory
            .set_stack_in_hand(hand, ItemStack::new(2, &Item::ENDER_PEARL));
        let observed = player.player.inventory.get_stack_in_hand(hand);
        fixture
            .server
            .item_registry
            .on_use(&observed, &player.player, hand);
        assert_eq!(
            player.player.inventory.get_stack_in_hand(hand).item_count,
            count
        );
        assert_eq!(
            world.entities.load().len(),
            if hand == Hand::Right { 1 } else { 2 }
        );
        assert!(
            player
                .player
                .has_item_cooldown(Item::ENDER_PEARL.registry_key)
        );
        // A different player is unnecessary; clear the first use's cooldown for the second hand.
        player.player.item_cooldowns.lock().unwrap().clear();
    }
    assert_eq!(
        player
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::ENDER_PEARL.id)),
        2
    );
    assert_eq!(calls.load(Relaxed), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_refund_throwables_when_spawn_is_cancelled() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(
            Arc::new(CancelSpawn(calls.clone())),
            EventPriority::Normal,
            true,
        );

    for item in [
        &Item::ENDER_PEARL,
        &Item::EGG,
        &Item::EXPERIENCE_BOTTLE,
        &Item::SNOWBALL,
        &Item::SPLASH_POTION,
        &Item::LINGERING_POTION,
        &Item::WIND_CHARGE,
    ] {
        for slot in 0..player.player.inventory.size() {
            player
                .player
                .inventory
                .set_stack(slot, ItemStack::EMPTY.clone());
        }
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(2, item));
        let observed = player.player.inventory.get_stack_in_hand(Hand::Right);
        fixture
            .server
            .item_registry
            .on_use(&observed, &player.player, Hand::Right);

        assert!(
            world.entities.load().is_empty(),
            "cancelled {item:?} spawned"
        );
        let refunded: u16 = (0..player.player.inventory.size())
            .map(|slot| player.player.inventory.get_stack(slot))
            .filter(|stack| stack.are_items_and_components_equal(&observed))
            .map(|stack| u16::from(stack.item_count))
            .sum();
        assert_eq!(refunded, 2, "cancelled {item:?} was not refunded");
        assert_eq!(
            player
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(item.id)),
            0,
            "cancelled {item:?} awarded its use statistic"
        );
        assert!(
            !player.player.has_item_cooldown(item.registry_key),
            "cancelled {item:?} started a cooldown"
        );
    }
    assert_eq!(calls.load(Relaxed), 7);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bucket_rejects_swapped_dispatch_snapshot() {
    use crate::{entity::EntityBase, world::spawn_test_support};
    use pumpkin_data::{Block, biome::Biome};
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let target = BlockPos::new(8, 65, 11);
    let destination = BlockPos::new(8, 65, 10);
    for (item, block) in [
        (&Item::WATER_BUCKET, &Block::STONE),
        (&Item::BUCKET, &Block::WATER),
    ] {
        world.set_block_state(
            &target,
            block.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        world.set_block_state(
            &destination,
            Block::AIR.default_state.id,
            pumpkin_world::world::BlockFlags::FORCE_STATE,
        );
        let original = ItemStack::new(1, item);
        let replacement = ItemStack::new(1, item);
        player.player.inventory.set_stack(0, original.clone());
        let chest = Arc::new(ChestBlockEntity::new(BlockPos::new(8, 64, 8)));
        chest.set_stack(0, replacement.clone());
        open_chest(&player, chest.clone());
        let mut swap = click();
        swap.mode = SlotActionType::Swap;
        player.player.on_slot_click(swap, &fixture.server);
        let moved = chest.get_stack(0);
        fixture.server.item_registry.on_use_with_rotation(
            &original,
            &player.player,
            0.0,
            0.0,
            Hand::Right,
        );
        assert_eq!(
            world.get_block(&target),
            block,
            "stale bucket use removed its fluid source"
        );
        assert_eq!(
            world.get_block(&destination),
            &Block::AIR,
            "stale bucket use placed free fluid"
        );
        let current = player.player.inventory.held_item();
        assert_eq!(current.uid, replacement.uid);
        assert!(current.are_equal(&replacement));
        assert_eq!(chest.get_stack(0).uid, moved.uid);
        assert!(chest.get_stack(0).are_equal(&original));
        assert_eq!(
            player
                .player
                .stats
                .lock()
                .unwrap()
                .get(StatisticCategory::Used, i32::from(item.id)),
            0
        );
    }
    fixture.server.shutdown().await;
}

struct SwapHandOnInteract(Arc<AtomicUsize>);
impl EventHandler<crate::plugin::api::events::player::player_interact_event::PlayerInteractEvent>
    for SwapHandOnInteract
{
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::player_interact_event::PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        if self.0.fetch_add(1, Relaxed) == 0 {
            let mut swap = click();
            swap.mode = SlotActionType::Swap;
            event.player.on_slot_click(swap, server);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_transactions_bedrock_pearl_packet_rejects_interact_callback_swap() {
    use crate::{
        net::bedrock::combat_test_support::TestBedrockPlayer,
        plugin::api::events::player::player_interact_event::PlayerInteractEvent,
    };
    use pumpkin_inventory::generic_container_screen_handler::create_generic_9x3;
    use pumpkin_protocol::{
        bedrock::{
            network_item::NetworkItemDescriptor,
            server::inventory_transaction::{
                HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
            },
        },
        codec::{var_int::VarInt, var_uint::VarUInt},
    };
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
    let fixture = DeathTestWorld::new().await;
    let bedrock = TestBedrockPlayer::new(&fixture.world()).await;
    let observed = ItemStack::new(2, &Item::ENDER_PEARL);
    bedrock.player.inventory.set_stack(0, observed.clone());
    let replacement = ItemStack::new(1, &Item::STONE);
    let chest = Arc::new(ChestBlockEntity::new(BlockPos::new(8, 64, 8)));
    chest.set_stack(0, replacement.clone());
    let handler = create_generic_9x3(
        1,
        &bedrock.player.inventory,
        chest.clone(),
        bedrock.player.as_ref(),
    );
    *bedrock.player.current_screen_handler.lock().unwrap() =
        Arc::new(pumpkin_util::sync::Mutex::new(handler));
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(
            Arc::new(SwapHandOnInteract(calls.clone())),
            EventPriority::Normal,
            true,
        );
    // Bedrock keeps stack_for_use across PlayerInteractEvent. The callback drives a real
    // number-key click between that packet snapshot and the item-use commit.
    bedrock.client().handle_inventory_action(
        &bedrock.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: false,
            actions: Vec::new(),
            transaction_type: VarUInt(2),
            transaction_data: TransactionData::UseItem(UseItemTransactionData {
                action_type: VarInt(1),
                trigger_type: 0,
                block_position: BlockPos::new(0, 0, 0),
                block_face: 0,
                hot_bar_slot: VarInt(0),
                hand: HandSlot::Mainhand,
                item_in_hand: NetworkItemDescriptor::from(&observed),
                player_position: Vector3::new(0.0, 0.0, 0.0),
                click_position: Vector3::new(0.0, 0.0, 0.0),
                block_runtime_id: VarUInt(0),
                client_prediction: 0,
                client_cooldown_state: 0,
            }),
        },
    );
    assert_eq!(calls.load(Relaxed), 1);
    assert!(fixture.world().entities.load().is_empty());
    let current = bedrock.player.inventory.held_item();
    assert_eq!(current.uid, replacement.uid);
    assert!(current.are_equal(&replacement));
    assert!(chest.get_stack(0).are_equal(&observed));
    assert_eq!(
        bedrock
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::ENDER_PEARL.id)),
        0
    );
    assert!(
        !bedrock
            .player
            .has_item_cooldown(Item::ENDER_PEARL.registry_key)
    );
    bedrock.close().await;
    fixture.server.shutdown().await;
}
