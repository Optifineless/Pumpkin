#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    biome::Biome,
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
    entity::EntityPose,
    item::Item,
    item_stack::ItemStack,
    world::WorldEvent,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::java::client::play::CWorldEvent;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use super::World;
use crate::{
    block::{
        BlockHitResult, blocks::player_destroy::test_support::PlayerFixture,
        entities::chest::ChestBlockEntity,
    },
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        api::events::{
            block::{block_break::BlockBreakEvent, block_physics::BlockPhysicsEvent},
            player::player_bed::PlayerBedLeaveEvent,
        },
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);

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
fn replacement_chest(world: &Arc<World>, position: BlockPos) {
    world.set_block_state(
        &position,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let chest = Arc::new(ChestBlockEntity::new(position));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    world.add_block_entity(chest);
}
fn assert_chest(world: &World, position: BlockPos) {
    assert_eq!(world.get_block(&position), &Block::CHEST);
    let stack = world
        .get_block_entity(&position)
        .unwrap()
        .get_inventory()
        .unwrap()
        .get_stack(0);
    assert_eq!(stack.item.id, Item::DIAMOND.id);
    assert_eq!(stack.item_count, 7);
}
fn sleeping(fixture: &PlayerFixture) -> TestPlayer {
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let mut props = WhiteBedLikeProperties::default(&Block::STRAW_BED);
    props.part = BedPart::Foot;
    props.facing = HorizontalFacing::East;
    fixture.world.set_block_state(
        &FOOT,
        props.to_state_id(&Block::STRAW_BED),
        BlockFlags::NOTIFY_ALL,
    );
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let client = TestPlayer::new(&fixture.world);
    client.player.get_entity().set_pos(FOOT.to_centered_f64());
    let cursor = Vector3::new(0.5, 1.0, 0.5);
    fixture.world.block_registry.on_use(
        &Block::STRAW_BED,
        &client.player,
        &FOOT,
        &BlockHitResult {
            face: &BlockDirection::Up,
            cursor_pos: &cursor,
        },
        &fixture.world.server.upgrade().unwrap(),
        &fixture.world,
    );
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    client
}
fn assert_awake(client: &TestPlayer) {
    assert!(client.player.sleeping_since.load().is_none());
    assert!(client.player.sleeping_bed_pos.load().is_none());
    assert!(client.player.get_entity().pose.load() == EntityPose::Standing);
}
fn item_count(world: &World, item: &Item) -> i32 {
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
fn destroy_packet(client: &TestPlayer, position: BlockPos, state: BlockStateId) -> bytes::Bytes {
    client
        .client()
        .serialize_packet(&CWorldEvent::new(
            WorldEvent::ParticlesDestroyBlock as i32,
            position,
            state.as_u16().into(),
            false,
        ))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn straw_leave_callback_preserves_replacement_chest_and_invalid_pair() {
    let fixture = PlayerFixture::new();
    let mut client = sleeping(&fixture);
    let world = fixture.world.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    register::<PlayerBedLeaveEvent>(&fixture.world, move |_| {
        seen.fetch_add(1, Relaxed);
        replacement_chest(&world, FOOT);
    });
    let head_state = fixture.world.get_block_state_id(&HEAD);
    let rejected = destroy_packet(&client, HEAD, head_state);
    client.take_packets();
    client.player.wake_up();
    assert_eq!(calls.load(Relaxed), 1);
    assert_awake(&client);
    assert_chest(&fixture.world, FOOT);
    assert_eq!(fixture.world.get_block_state_id(&HEAD), head_state);
    assert!(!client.take_packets().contains(&rejected));
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 0);
    assert_eq!(item_count(&fixture.world, &Item::STRAW_BED), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn straw_break_callbacks_preserve_replacements_and_cancellation() {
    // Replace own head, replace partner from head event, replace partner from its own event,
    // cancel head, cancel partner. Each callback runs inside the real wake operation.
    for action in 0..5 {
        let fixture = PlayerFixture::new();
        let mut client = sleeping(&fixture);
        let world = fixture.world.clone();
        let head_state = world.get_block_state_id(&HEAD);
        let foot_state = world.get_block_state_id(&FOOT);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        register::<BlockBreakEvent>(&fixture.world, move |event| {
            assert_eq!(event.block, &Block::STRAW_BED);
            assert!(event.player.is_none());
            assert_eq!(event.exp, 0);
            assert!(!event.drop);
            seen.fetch_add(1, Relaxed);
            let target = if action == 2 || action == 4 {
                FOOT
            } else {
                HEAD
            };
            if event.block_position == target {
                match action {
                    0 => replacement_chest(&world, HEAD),
                    1 | 2 => replacement_chest(&world, FOOT),
                    _ => event.cancelled = true,
                }
            }
        });
        let head_packet = destroy_packet(&client, HEAD, head_state);
        let foot_packet = destroy_packet(&client, FOOT, foot_state);
        client.take_packets();
        client.player.wake_up();
        assert_awake(&client);
        let packets = client.take_packets();
        assert!(
            !packets.contains(&foot_packet),
            "rejected partner action={action}"
        );
        if action == 0 || action == 3 {
            assert!(
                !packets.contains(&head_packet),
                "rejected head action={action}"
            );
            assert_eq!(fixture.world.get_block_state_id(&FOOT), foot_state);
        } else {
            assert!(
                packets.contains(&head_packet),
                "accepted head action={action}"
            );
            assert!(fixture.world.get_block(&HEAD).is_air());
        }
        match action {
            0 => assert_chest(&fixture.world, HEAD),
            1 | 2 => assert_chest(&fixture.world, FOOT),
            3 => assert_eq!(fixture.world.get_block_state_id(&HEAD), head_state),
            _ => assert_eq!(fixture.world.get_block_state_id(&FOOT), foot_state),
        }
        assert_eq!(
            calls.load(Relaxed),
            if action == 2 || action == 4 { 2 } else { 1 }
        );
        assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 0);
        assert_eq!(item_count(&fixture.world, &Item::STRAW_BED), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn straw_committed_break_keeps_completion_effects_after_neighbor_replacement() {
    for drop in [false, true] {
        let fixture = PlayerFixture::new();
        let mut client = sleeping(&fixture);
        let head_state = fixture.world.get_block_state_id(&HEAD);
        let foot_state = fixture.world.get_block_state_id(&FOOT);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let world = fixture.world.clone();
        register::<BlockPhysicsEvent>(&fixture.world, move |event| {
            if event.changed_pos == FOOT && seen.fetch_add(1, Relaxed) == 0 {
                assert!(world.get_block(&HEAD).is_air());
                replacement_chest(&world, HEAD);
            }
        });
        register::<BlockBreakEvent>(&fixture.world, move |event| event.drop = drop);
        let head_packet = destroy_packet(&client, HEAD, head_state);
        let foot_packet = destroy_packet(&client, FOOT, foot_state);
        client.take_packets();
        client.player.wake_up();
        assert_awake(&client);
        assert!(calls.load(Relaxed) > 0);
        assert_chest(&fixture.world, HEAD);
        assert!(fixture.world.get_block(&FOOT).is_air());
        let packets = client.take_packets();
        assert_eq!(
            packets
                .iter()
                .filter(|packet| **packet == head_packet)
                .count(),
            1
        );
        assert_eq!(
            packets
                .iter()
                .filter(|packet| **packet == foot_packet)
                .count(),
            1
        );
        assert_eq!(
            item_count(&fixture.world, &Item::STRAW_BED),
            i32::from(drop)
        );
        assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn straw_cleanup_sends_bedrock_particles_only_for_committed_removals() {
    for cancel in [false, true] {
        let fixture = PlayerFixture::new();
        let java = sleeping(&fixture);
        let state = fixture.world.get_block_state_id(&HEAD);
        let mut bedrock =
            crate::net::bedrock::combat_test_support::TestBedrockPlayer::new(&fixture.world).await;
        bedrock
            .player
            .watched_section
            .store(java.player.watched_section.load());
        fixture
            .world
            .players
            .store(Arc::new(vec![java.player.clone(), bedrock.player.clone()]));
        register::<BlockBreakEvent>(&fixture.world, move |event| event.cancelled = cancel);
        let expected = bedrock.client().serialize_packet(
            &pumpkin_protocol::bedrock::client::level_event::CLevelEvent {
                event_id: pumpkin_protocol::VarInt(pumpkin_protocol::bedrock::client::level_event::LevelEvent::ParticlesDestroyBlock as i32),
                position: HEAD.to_centered_f64().to_f32_lossy(),
                data: pumpkin_protocol::VarInt(pumpkin_data::BlockState::to_be_network_id(state) as i32),
            },
        ).unwrap();
        bedrock.take_packets();
        java.player.wake_up();
        assert_awake(&java);
        assert_eq!(bedrock.take_packets().contains(&expected), !cancel);
        assert_eq!(fixture.world.get_block(&HEAD).is_air(), !cancel);
        assert_eq!(item_count(&fixture.world, &Item::STRAW_BED), 0);
        bedrock.close().await;
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn straw_head_break_callback_preserves_changed_straw_partner_and_rotated_pair() {
    for rotated in [false, true] {
        let fixture = PlayerFixture::new();
        let mut client = sleeping(&fixture);
        let world = fixture.world.clone();
        let original_foot = world.get_block_state_id(&FOOT);
        let mut props = WhiteBedLikeProperties::from_state_id(original_foot);
        props.occupied = false;
        if rotated {
            props.facing = HorizontalFacing::South;
        }
        let replacement = props.to_state_id(&Block::STRAW_BED);
        let new_head = FOOT.offset(props.facing.to_offset());
        props.part = BedPart::Head;
        let new_head_state = props.to_state_id(&Block::STRAW_BED);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        register::<BlockBreakEvent>(&fixture.world, move |event| {
            assert_eq!(event.block_position, HEAD);
            seen.fetch_add(1, Relaxed);
            let flags = BlockFlags::NOTIFY_LISTENERS
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;
            world.set_block_state(&FOOT, replacement, flags);
            if rotated {
                world.set_block_state(&new_head, new_head_state, flags);
            }
        });
        let rejected = destroy_packet(&client, FOOT, replacement);
        client.take_packets();
        client.player.wake_up();
        assert_awake(&client);
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(fixture.world.get_block_state_id(&FOOT), replacement);
        assert!(fixture.world.get_block(&HEAD).is_air());
        if rotated {
            assert_eq!(fixture.world.get_block_state_id(&new_head), new_head_state);
        }
        assert!(!client.take_packets().contains(&rejected));
        assert_eq!(item_count(&fixture.world, &Item::STRAW_BED), 0);
        fixture.finish().await;
    }
}
