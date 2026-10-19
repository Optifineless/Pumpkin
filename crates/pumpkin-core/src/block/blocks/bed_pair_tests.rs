#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{BedPart, WhiteBedLikeProperties},
    item::Item,
    item_stack::ItemStack,
    world::WorldEvent,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::{
        client::play::CWorldEvent,
        server::play::{SPlayerAction, SUseItemOn},
    },
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
    sync::{LockClass, Mutex},
};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{
        blocks::player_destroy::test_support::PlayerFixture, entities::chest::ChestBlockEntity,
    },
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        api::events::block::{
            block_break::BlockBreakEvent, block_drop_item::BlockDropItemEvent,
            block_physics::BlockPhysicsEvent,
        },
    },
    server::Server,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);

#[path = "bed_pair_review_tests.rs"]
mod review;

#[path = "bed_pair_round3_tests.rs"]
mod round3;

enum TestRecords {}
impl LockClass for TestRecords {
    const NAME: &'static str = "bed test records";
    const LEVEL: u8 = 40;
}

struct Callback<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn register<E: Payload + Send + Sync + 'static>(
    world: &World,
    callback: impl Fn(&mut E) + Send + Sync + 'static,
) {
    world
        .server
        .upgrade()
        .unwrap()
        .plugin_manager
        .register::<E, _>(Arc::new(Callback(callback)), EventPriority::Normal, true);
}

fn place(fixture: &PlayerFixture, item: &'static Item) -> TestPlayer {
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let client = TestPlayer::new(&fixture.world);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 2.5));
    client.player.get_entity().set_rotation(-90.0, 0.0);
    client
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(1, item));
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: FOOT.down(),
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.world.server.upgrade().unwrap(),
        )
        .unwrap();
    for (position, part) in [(FOOT, BedPart::Foot), (HEAD, BedPart::Head)] {
        let (block, state) = fixture.world.get_block_and_state_id(&position);
        assert_eq!(block.item_id, item.id);
        assert_eq!(WhiteBedLikeProperties::from_state_id(state).part, part);
    }
    assert!(client.player.inventory().held_item().is_empty());
    client
}

fn destroy(client: &TestPlayer, world: &World, position: BlockPos) {
    let server = world.server.upgrade().unwrap();
    for status in [0, 2] {
        client.client().handle_player_action(
            &client.player,
            &SPlayerAction {
                status: VarInt(status),
                position,
                face: 1,
                sequence: VarInt(status + 2),
            },
            &server,
        );
        if client.player.gamemode.load() == GameMode::Creative {
            break;
        }
    }
}

fn drops(world: &World, item: &Item) -> i32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            if stack.item.id == item.id {
                i32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

async fn finish(fixture: PlayerFixture) {
    tokio::time::timeout(std::time::Duration::from_secs(20), fixture.finish())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_head_loot_cancellation_observes_the_bed() {
    let fixture = PlayerFixture::new();
    let client = place(&fixture, &Item::RED_BED);
    let seen = Arc::new(AtomicBool::new(false));
    let observed = seen.clone();
    register::<BlockDropItemEvent>(&fixture.world, move |event| {
        assert_eq!(event.block_pos, HEAD);
        if event.world.get_block(&event.block_pos) == &Block::RED_BED {
            observed.store(true, Relaxed);
            event.cancelled = true;
        }
    });
    destroy(&client, &fixture.world, HEAD);
    assert!(
        seen.load(Relaxed),
        "loot listeners must observe the original bed"
    );
    assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
    assert!(fixture.world.get_block(&FOOT).is_air());
    assert!(fixture.world.get_block(&HEAD).is_air());
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_survival_java_either_bed_half_drops_once() {
    for item in [&Item::RED_BED, &Item::STRAW_BED] {
        for position in [FOOT, HEAD] {
            let fixture = PlayerFixture::new();
            let client = place(&fixture, item);
            let notifications = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
            let seen = notifications.clone();
            register::<BlockPhysicsEvent>(&fixture.world, move |event| {
                seen.lock().unwrap().push(event.changed_pos);
            });
            destroy(&client, &fixture.world, position);
            assert_eq!(
                drops(&fixture.world, item),
                1,
                "{} at {position:?}",
                item.registry_key
            );
            assert!(client.player.inventory().held_item().is_empty());
            assert!(fixture.world.get_block(&FOOT).is_air());
            assert!(fixture.world.get_block(&HEAD).is_air());
            for half in [FOOT, HEAD] {
                assert!(
                    notifications.lock().unwrap().contains(&half),
                    "missing notification for {half:?}"
                );
            }
            finish(fixture).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_creative_java_either_bed_half_drops_none() {
    for item in [&Item::RED_BED, &Item::STRAW_BED] {
        for position in [FOOT, HEAD] {
            let fixture = PlayerFixture::new();
            let mut client = place(&fixture, item);
            let mut observer = TestPlayer::new(&fixture.world);
            fixture.world.players.store(Arc::new(vec![
                client.player.clone(),
                observer.player.clone(),
            ]));
            let head_effect = client
                .client()
                .serialize_packet(&CWorldEvent::new(
                    WorldEvent::ParticlesDestroyBlock as i32,
                    HEAD,
                    i32::from(fixture.world.get_block_state_id(&HEAD).as_u16()),
                    false,
                ))
                .unwrap();
            client.take_packets();
            observer.take_packets();
            client.player.gamemode.store(GameMode::Creative);
            destroy(&client, &fixture.world, position);
            assert_eq!(drops(&fixture.world, item), 0);
            assert!(fixture.world.get_block(&FOOT).is_air());
            assert!(fixture.world.get_block(&HEAD).is_air());
            assert!(
                !client.take_packets().contains(&head_effect),
                "Java predicts the head's break effect"
            );
            assert_eq!(
                observer
                    .take_packets()
                    .iter()
                    .filter(|packet| **packet == head_effect)
                    .count(),
                1
            );
            finish(fixture).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_cancelled_and_protected_java_break_keep_both_halves() {
    for protected in [false, true] {
        for mode in [GameMode::Survival, GameMode::Creative] {
            let fixture = PlayerFixture::new();
            let client = place(&fixture, &Item::RED_BED);
            client.player.gamemode.store(mode);
            let before = [
                fixture.world.get_block_state_id(&FOOT),
                fixture.world.get_block_state_id(&HEAD),
            ];
            let breaks = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
            let seen = breaks.clone();
            register::<BlockBreakEvent>(&fixture.world, move |event| {
                seen.lock().unwrap().push(event.block_position);
                event.cancelled = true;
            });
            if protected {
                fixture
                    .world
                    .server
                    .upgrade()
                    .unwrap()
                    .data
                    .operator_config
                    .write()
                    .unwrap()
                    .ops
                    .push(pumpkin_config::op::Op::new(
                        uuid::Uuid::new_v4(),
                        "operator".into(),
                        PermissionLvl::Four,
                        false,
                    ));
                assert!(fixture.world.is_in_spawn_protection(&client.player, &FOOT));
            }
            destroy(&client, &fixture.world, FOOT);
            assert_eq!(
                [
                    fixture.world.get_block_state_id(&FOOT),
                    fixture.world.get_block_state_id(&HEAD)
                ],
                before
            );
            assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
            assert_eq!(breaks.lock().unwrap().len(), usize::from(!protected));
            finish(fixture).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_partner_break_veto_keeps_the_cancelled_half() {
    for mode in [GameMode::Survival, GameMode::Creative] {
        for position in [FOOT, HEAD] {
            let fixture = PlayerFixture::new();
            let client = place(&fixture, &Item::RED_BED);
            client.player.gamemode.store(mode);
            let partner = if position == FOOT { HEAD } else { FOOT };
            let expected = fixture.world.get_block_state_id(&partner);
            let breaks = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
            let seen = breaks.clone();
            register::<BlockBreakEvent>(&fixture.world, move |event| {
                seen.lock().unwrap().push(event.block_position);
                event.cancelled = event.block_position == partner;
            });
            destroy(&client, &fixture.world, position);
            assert_eq!(*breaks.lock().unwrap(), [position, partner]);
            assert!(fixture.world.get_block(&position).is_air());
            assert_eq!(fixture.world.get_block_state_id(&partner), expected);
            assert_eq!(
                drops(&fixture.world, &Item::RED_BED),
                i32::from(mode == GameMode::Survival && position == HEAD)
            );
            finish(fixture).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_bed_drop_overrides_and_event_order_are_preserved() {
    for drop_items in [false, true] {
        for position in [FOOT, HEAD] {
            let fixture = PlayerFixture::new();
            let client = place(&fixture, &Item::RED_BED);
            // An explicit plugin override may enable Creative drops, as on the ordinary break path.
            if drop_items {
                client.player.gamemode.store(GameMode::Creative);
            }
            let sequence = Arc::new(Mutex::<_, TestRecords>::new(Vec::new()));
            let events = sequence.clone();
            register::<BlockBreakEvent>(&fixture.world, move |event| {
                event.drop = drop_items;
                events
                    .lock()
                    .unwrap()
                    .push((event.block_position, event.player.is_some(), false));
            });
            let events = sequence.clone();
            register::<BlockDropItemEvent>(&fixture.world, move |event| {
                events.lock().unwrap().push((event.block_pos, false, true));
            });
            destroy(&client, &fixture.world, position);
            assert_eq!(drops(&fixture.world, &Item::RED_BED), i32::from(drop_items));
            let partner = if position == FOOT { HEAD } else { FOOT };
            let mut expected = vec![(position, true, false)];
            if position == HEAD && drop_items {
                expected.push((HEAD, false, true));
            }
            expected.push((partner, true, false));
            if position == FOOT && drop_items {
                expected.push((HEAD, false, true));
            }
            assert_eq!(*sequence.lock().unwrap(), expected);
            assert!(fixture.world.get_block(&FOOT).is_air());
            assert!(fixture.world.get_block(&HEAD).is_air());
            finish(fixture).await;
        }
    }
}

fn install_chest(world: &Arc<World>, position: BlockPos) {
    world.set_block_state(
        &position,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let chest = Arc::new(ChestBlockEntity::new(position));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    world.add_block_entity(chest);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_break_callback_cannot_remove_a_replacement_container() {
    for replacement in [FOOT, HEAD] {
        let fixture = PlayerFixture::new();
        let client = place(&fixture, &Item::RED_BED);
        let world = fixture.world.clone();
        register::<BlockBreakEvent>(&fixture.world, move |event| {
            if event.block_position == replacement {
                install_chest(&world, replacement);
            }
        });
        destroy(&client, &fixture.world, FOOT);
        assert_eq!(fixture.world.get_block(&replacement), &Block::CHEST);
        let stack = fixture
            .world
            .get_block_entity(&replacement)
            .unwrap()
            .get_inventory()
            .unwrap()
            .get_stack(0);
        assert_eq!(stack.item.id, Item::DIAMOND.id);
        assert_eq!(stack.item_count, 7);
        assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
        finish(fixture).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_physics_callback_preserves_new_bed_pair() {
    for item in [&Item::RED_BED, &Item::STRAW_BED] {
        let fixture = PlayerFixture::new();
        let client = place(&fixture, item);
        let block = fixture.world.get_block(&FOOT);
        let mut replacement =
            WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&FOOT));
        replacement.occupied = true;
        let foot_state = replacement.to_state_id(block);
        replacement.part = BedPart::Head;
        let head_state = replacement.to_state_id(block);
        let world = fixture.world.clone();
        let installed = Arc::new(AtomicBool::new(false));
        let seen = installed.clone();
        register::<BlockPhysicsEvent>(&fixture.world, move |event| {
            if event.changed_pos == FOOT && !seen.swap(true, Relaxed) {
                let flags = BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE;
                world.set_block_state(&FOOT, foot_state, flags);
                world.set_block_state(&HEAD, head_state, flags);
            }
        });
        destroy(&client, &fixture.world, FOOT);
        assert!(installed.load(Relaxed));
        assert_eq!(fixture.world.get_block_state_id(&FOOT), foot_state);
        assert_eq!(fixture.world.get_block_state_id(&HEAD), head_state);
        assert_eq!(drops(&fixture.world, item), 0);
        finish(fixture).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_loot_callback_preserves_replacements_without_stale_drops() {
    for replacement in 0..3 {
        let fixture = PlayerFixture::new();
        let client = place(&fixture, &Item::RED_BED);
        let foot_state = fixture.world.get_block_state_id(&FOOT);
        let mut props = WhiteBedLikeProperties::from_state_id(foot_state);
        props.occupied = true;
        let new_foot = props.to_state_id(&Block::RED_BED);
        props.part = BedPart::Head;
        let new_head = props.to_state_id(&Block::RED_BED);
        let world = fixture.world.clone();
        register::<BlockDropItemEvent>(&fixture.world, move |event| {
            assert_eq!(event.block_pos, HEAD);
            assert_eq!(world.get_block(&HEAD), &Block::RED_BED);
            let flags = BlockFlags::NOTIFY_LISTENERS
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;
            if replacement == 1 {
                world.set_block_state(&HEAD, new_head, flags);
            } else {
                install_chest(&world, HEAD);
            }
            if replacement == 2 {
                world.set_block_state(&FOOT, new_foot, flags);
            }
        });
        destroy(&client, &fixture.world, HEAD);
        if replacement == 1 {
            assert_eq!(fixture.world.get_block_state_id(&HEAD), new_head);
            assert_eq!(fixture.world.get_block_state_id(&FOOT), foot_state);
        } else {
            assert_eq!(fixture.world.get_block(&HEAD), &Block::CHEST);
            let stack = fixture
                .world
                .get_block_entity(&HEAD)
                .unwrap()
                .get_inventory()
                .unwrap()
                .get_stack(0);
            assert_eq!(stack.item.id, Item::DIAMOND.id);
            assert_eq!(stack.item_count, 7);
            if replacement == 0 {
                assert!(fixture.world.get_block(&FOOT).is_air());
            } else {
                assert_eq!(fixture.world.get_block_state_id(&FOOT), new_foot);
            }
        }
        assert_eq!(drops(&fixture.world, &Item::RED_BED), 0);
        finish(fixture).await;
    }
}
