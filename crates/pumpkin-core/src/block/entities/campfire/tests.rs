use super::*;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::block::campfire_start::CampfireStartEvent,
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::GameMode;
use pumpkin_world::world::BlockFlags;
use std::{future::Future, sync::atomic::AtomicUsize, time::Duration};

mod cooldown_tests;

const POS: BlockPos = BlockPos::new(8, 65, 8);

async fn bounded(test: impl Future<Output = ()>) {
    tokio::time::timeout(Duration::from_secs(180), test)
        .await
        .unwrap();
}

async fn setup(block: &Block, lit: bool) -> (DeathTestWorld, TestPlayer, Arc<CampfireBlockEntity>) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut chunk = proto(&Biome::PLAINS, &Block::STONE);
    let mut props = CampfireLikeProperties::from_state_id(block.default_state.id);
    props.lit = lit;
    chunk.set_block_state(8, 65, 8, props.to_state_id(block).to_state());
    publish(&world, chunk);
    let campfire = Arc::new(CampfireBlockEntity::new(POS));
    world.add_block_entity(campfire.clone());
    let client = TestPlayer::new(&world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 9.5));
    (fixture, client, campfire)
}

fn use_food(fixture: &DeathTestWorld, client: &TestPlayer, hand: i32) {
    use_food_on_face(fixture, client, hand, 1);
}

fn use_food_on_face(fixture: &DeathTestWorld, client: &TestPlayer, hand: i32, face: i32) {
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(hand),
                position: POS,
                face: VarInt(face),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.server,
        )
        .unwrap();
}

fn set_lit(world: &Arc<World>, lit: bool) {
    let block = world.get_block(&POS);
    let mut props = CampfireLikeProperties::from_state_id(world.get_block_state_id(&POS));
    props.lit = lit;
    world.set_block_state(&POS, props.to_state_id(block), BlockFlags::empty());
}

fn dropped(world: &World, kind: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            entity
                .cast_any()
                .downcast_ref::<ItemEntity>()
                .map(|item| item.get_item_stack().lock().unwrap().clone())
        })
        .filter(|stack| stack.item == kind)
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

enum Action {
    Cancel,
    Duration(i32),
    ReplaceSource,
    OccupySlot,
    ReplaceCampfire,
}

struct StartHandler {
    calls: Arc<AtomicUsize>,
    player: Arc<Player>,
    campfire: Arc<CampfireBlockEntity>,
    source: usize,
    action: Action,
}

impl EventHandler<CampfireStartEvent> for StartHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut CampfireStartEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(event.block_pos, POS);
        assert!([&Item::BEEF, &Item::KELP].contains(&event.item.item));
        assert_eq!(event.item.item_count, 1);
        assert_eq!(event.slot, 0);
        // A different thread must acquire both owners while the real plugin callback runs.
        let source = self.player.inventory().slot_transaction(self.source);
        let destination = self.campfire.slot_transaction(0);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = send.send(source.try_lock().is_some() && destination.try_lock().is_some());
        });
        assert!(receive.recv_timeout(Duration::from_secs(2)).unwrap());
        match self.action {
            Action::Cancel => event.cancelled = true,
            Action::Duration(time) => event.cooking_time = time,
            Action::ReplaceSource => self
                .player
                .inventory()
                .set_stack(self.source, ItemStack::new(2, &Item::BEEF)),
            Action::OccupySlot => self
                .campfire
                .set_stack(0, ItemStack::new(1, &Item::PORKCHOP)),
            Action::ReplaceCampfire => {
                event
                    .world
                    .set_block_state(&POS, Block::AIR.default_state.id, BlockFlags::empty());
                event.world.set_block_state(
                    &POS,
                    Block::CAMPFIRE.default_state.id,
                    BlockFlags::empty(),
                );
                event
                    .world
                    .add_block_entity(Arc::new(CampfireBlockEntity::new(POS)));
            }
        }
        Box::pin(async {})
    }
}

fn register(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    campfire: &Arc<CampfireBlockEntity>,
    source: usize,
    action: Action,
) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<CampfireStartEvent, _>(
            Arc::new(StartHandler {
                calls: calls.clone(),
                player: client.player.clone(),
                campfire: campfire.clone(),
                source,
                action,
            }),
            EventPriority::Normal,
            true,
        );
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_unlit_storage_both_hands_modes_and_later_cooking() {
    bounded(async {
        for block in [&Block::CAMPFIRE, &Block::SOUL_CAMPFIRE] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                for (hand, source) in [(0, 0), (1, 40)] {
                    let (fixture, client, campfire) = setup(block, false).await;
                    let world = fixture.world();
                    client.player.gamemode.store(mode);
                    client
                        .player
                        .inventory()
                        .set_stack(source, ItemStack::new(2, &Item::BEEF));
                    use_food(&fixture, &client, hand);
                    assert_eq!(
                        campfire.get_stack(0).item_count,
                        1,
                        "#85 unlit campfire must accept food"
                    );
                    assert!((1..4).all(|slot| campfire.get_stack(slot).is_empty()));
                    assert_eq!(
                        client.player.inventory().get_stack(source).item_count,
                        if mode == GameMode::Creative { 2 } else { 1 }
                    );
                    let duration = *campfire.cooking_total_times[0].lock().unwrap();
                    assert!(duration > 0);
                    for _ in 0..4 {
                        campfire.tick(&world);
                    }
                    assert_eq!(*campfire.cooking_times[0].lock().unwrap(), 0);
                    assert_eq!(dropped(&world, &Item::COOKED_BEEF), 0);
                    set_lit(&world, true);
                    for _ in 0..duration - 1 {
                        campfire.tick(&world);
                    }
                    assert_eq!(campfire.get_stack(0).item_count, 1);
                    campfire.tick(&world);
                    assert!(campfire.is_empty());
                    assert_eq!(dropped(&world, &Item::COOKED_BEEF), 1);
                    fixture.server.shutdown().await;
                }
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_invalid_and_full_campfires_do_not_consume_or_start() {
    bounded(async {
        let (fixture, client, campfire) = setup(&Block::CAMPFIRE, true).await;
        let calls = register(&fixture, &client, &campfire, 0, Action::Cancel);
        let invalid = ItemStack::new(2, &Item::DIAMOND);
        client.player.inventory().set_stack(0, invalid.clone());
        use_food(&fixture, &client, 0);
        assert!(client.player.inventory().get_stack(0).are_equal(&invalid));
        assert!(campfire.is_empty());
        for slot in 0..4 {
            campfire.set_stack(slot, ItemStack::new(1, &Item::PORKCHOP));
        }
        let beef = ItemStack::new(2, &Item::BEEF);
        client.player.inventory().set_stack(0, beef.clone());
        use_food(&fixture, &client, 0);
        assert!(client.player.inventory().get_stack(0).are_equal(&beef));
        assert!((0..4).all(|slot| campfire.get_stack(slot).item == &Item::PORKCHOP));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        fixture.server.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_start_cancellation_preserves_input_and_storage() {
    bounded(async {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for (hand, source) in [(0, 0), (1, 40)] {
                let (fixture, client, campfire) = setup(&Block::CAMPFIRE, true).await;
                client.player.gamemode.store(mode);
                let input = ItemStack::new(2, &Item::BEEF);
                client.player.inventory().set_stack(source, input.clone());
                let calls = register(&fixture, &client, &campfire, source, Action::Cancel);
                use_food(&fixture, &client, hand);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "#86 ordinary positive-duration insertion must fire start event"
                );
                assert!(
                    campfire.is_empty(),
                    "cancelled admission must not store food"
                );
                let remaining = client.player.inventory().get_stack(source);
                assert_eq!(remaining.uid, input.uid);
                assert!(remaining.are_equal(&input));
                for _ in 0..4 {
                    campfire.tick(&fixture.world());
                }
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(dropped(&fixture.world(), &Item::COOKED_BEEF), 0);
                fixture.server.shutdown().await;
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_cancelled_or_full_kelp_use_cannot_fall_through_to_placement() {
    bounded(async {
        let (fixture, client, _) = setup(&Block::CAMPFIRE, true).await;
        let world = fixture.world();
        let water = POS.east();
        world.set_block_state(
            &water.down(),
            Block::STONE.default_state.id,
            BlockFlags::empty(),
        );
        world.set_block_state(&water, Block::WATER.default_state.id, BlockFlags::empty());
        world.set_block_state(
            &water.up(),
            Block::WATER.default_state.id,
            BlockFlags::empty(),
        );
        world.set_block_state(&POS, Block::STONE.default_state.id, BlockFlags::empty());
        client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(2, &Item::KELP));
        use_food_on_face(&fixture, &client, 0, 5);
        assert_eq!(
            world.get_block(&water),
            &Block::KELP,
            "positive control: adjacent water permits kelp placement"
        );
        assert_eq!(client.player.inventory().get_stack(0).item_count, 1);
        world.set_block_state(&water, Block::WATER.default_state.id, BlockFlags::empty());
        world.set_block_state(&POS, Block::CAMPFIRE.default_state.id, BlockFlags::empty());
        let campfire = Arc::new(CampfireBlockEntity::new(POS));
        world.add_block_entity(campfire.clone());
        client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(2, &Item::KELP));
        let calls = register(&fixture, &client, &campfire, 0, Action::Cancel);
        use_food_on_face(&fixture, &client, 0, 5);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(world.get_block(&water), &Block::WATER);
        assert_eq!(client.player.inventory().get_stack(0).item_count, 2);
        assert!(campfire.is_empty());
        for slot in 0..4 {
            campfire.set_stack(slot, ItemStack::new(1, &Item::PORKCHOP));
        }
        use_food_on_face(&fixture, &client, 0, 5);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(world.get_block(&water), &Block::WATER);
        assert_eq!(client.player.inventory().get_stack(0).item_count, 2);
        fixture.server.shutdown().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_adjusted_duration_controls_real_completion_once() {
    bounded(async {
        for mode in [GameMode::Survival, GameMode::Creative] {
            for (hand, source) in [(0, 0), (1, 40)] {
                let (fixture, client, campfire) = setup(&Block::CAMPFIRE, true).await;
                let world = fixture.world();
                client.player.gamemode.store(mode);
                client
                    .player
                    .inventory()
                    .set_stack(source, ItemStack::new(2, &Item::BEEF));
                let calls = register(&fixture, &client, &campfire, source, Action::Duration(3));
                use_food(&fixture, &client, hand);
                assert_eq!(
                    *campfire.cooking_total_times[0].lock().unwrap(),
                    3,
                    "#86 start duration override must be committed"
                );
                for _ in 0..2 {
                    campfire.tick(&world);
                }
                assert_eq!(dropped(&world, &Item::COOKED_BEEF), 0);
                assert_eq!(campfire.get_stack(0).item_count, 1);
                campfire.tick(&world);
                assert!(campfire.is_empty());
                assert_eq!(dropped(&world, &Item::COOKED_BEEF), 1);
                campfire.tick(&world);
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                fixture.server.shutdown().await;
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_callbacks_cannot_replace_source_or_occupy_destination() {
    bounded(async {
        for action in [
            Action::ReplaceSource,
            Action::OccupySlot,
            Action::ReplaceCampfire,
        ] {
            let (fixture, client, campfire) = setup(&Block::CAMPFIRE, true).await;
            let source = ItemStack::new(2, &Item::BEEF);
            client.player.inventory().set_stack(40, source.clone());
            let occupied = matches!(action, Action::OccupySlot);
            let replaced_source = matches!(action, Action::ReplaceSource);
            let replaced_campfire = matches!(action, Action::ReplaceCampfire);
            let calls = register(&fixture, &client, &campfire, 40, action);
            use_food(&fixture, &client, 1);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            let remaining = client.player.inventory().get_stack(40);
            assert_eq!(
                remaining.item_count, 2,
                "callback invalidation must not pay input"
            );
            assert_eq!(remaining.uid == source.uid, !replaced_source);
            assert_eq!(campfire.get_stack(0).item == &Item::PORKCHOP, occupied);
            assert!(
                (1..4).all(|slot| campfire.get_stack(slot).is_empty()),
                "must not retry another slot after callback"
            );
            if replaced_campfire {
                let current = fixture.world().get_block_entity(&POS).unwrap();
                assert!(
                    current
                        .as_any()
                        .downcast_ref::<CampfireBlockEntity>()
                        .unwrap()
                        .is_empty()
                );
                assert!(!campfire.slot_transaction(0).is_live());
            }
            assert_eq!(dropped(&fixture.world(), &Item::BEEF), 0);
            fixture.server.shutdown().await;
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_nonpositive_admission_survives_nbt_reload_without_refiring() {
    bounded(async {
        for duration in [0, -2] {
            let (fixture, client, campfire) = setup(&Block::CAMPFIRE, false).await;
            let world = fixture.world();
            client
                .player
                .inventory()
                .set_stack(0, ItemStack::new(2, &Item::BEEF));
            let calls = register(&fixture, &client, &campfire, 0, Action::Duration(duration));
            use_food(&fixture, &client, 0);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            let mut saved = NbtCompound::new();
            campfire.write_internal(&mut saved);
            assert_eq!(
                saved.get_int_array("CookingTotalTimes").unwrap()[0],
                duration
            );
            let loaded = Arc::new(CampfireBlockEntity::from_nbt(&saved, POS));
            world.add_block_entity(loaded.clone());
            loaded.tick(&world);
            assert_eq!(loaded.get_stack(0).item_count, 1);
            assert_eq!(*loaded.cooking_times[0].lock().unwrap(), 0);
            set_lit(&world, true);
            loaded.tick(&world);
            assert!(loaded.is_empty());
            assert_eq!(dropped(&world, &Item::COOKED_BEEF), 1);
            assert_eq!(
                calls.load(Ordering::Relaxed),
                1,
                "persisted admission must suppress fallback"
            );
            fixture.server.shutdown().await;
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g14_legacy_nbt_preserves_progress_cooldown_and_fallback_cancellation() {
    bounded(async {
        let (fixture, client, campfire) = setup(&Block::CAMPFIRE, false).await;
        let world = fixture.world();
        let mut item = NbtCompound::new();
        ItemStack::new(1, &Item::BEEF).write_item_stack(&mut item);
        item.put_byte("Slot", 0);
        let mut saved = NbtCompound::new();
        saved.put_list("Items", vec![NbtTag::Compound(item)]);
        saved.put("CookingTimes", NbtTag::IntArray(vec![5, 0, 0, 0]));
        saved.put("CookingTotalTimes", NbtTag::IntArray(vec![6, 0, 0, 0]));
        let loaded = Arc::new(CampfireBlockEntity::from_nbt(&saved, POS));
        world.add_block_entity(loaded.clone());
        let calls = register(&fixture, &client, &campfire, 0, Action::Cancel);
        assert_eq!(*loaded.cooking_times[0].lock().unwrap(), 5);
        loaded.tick(&world);
        assert_eq!(
            *loaded.cooking_times[0].lock().unwrap(),
            3,
            "unlit cooldown removes two ticks"
        );
        set_lit(&world, true);
        for _ in 0..2 {
            loaded.tick(&world);
        }
        assert_eq!(loaded.get_stack(0).item_count, 1);
        loaded.tick(&world);
        assert!(loaded.is_empty());
        assert_eq!(dropped(&world, &Item::COOKED_BEEF), 1);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "positive legacy total must keep its meaning"
        );
        saved.put("CookingTimes", NbtTag::IntArray(vec![0; 4]));
        saved.put("CookingTotalTimes", NbtTag::IntArray(vec![0; 4]));
        let fallback = Arc::new(CampfireBlockEntity::from_nbt(&saved, POS));
        world.add_block_entity(fallback.clone());
        fallback.tick(&world);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(*fallback.cooking_times[0].lock().unwrap(), 0);
        assert_eq!(fallback.get_stack(0).item_count, 1);
        fixture.server.shutdown().await;
    })
    .await;
}
