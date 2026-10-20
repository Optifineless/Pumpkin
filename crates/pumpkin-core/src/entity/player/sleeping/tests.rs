#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection, biome::Biome, block_properties::WhiteBedLikeProperties,
    entity::EntityPose, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, SUseItemOn},
};
use pumpkin_util::{
    GameMode, PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::blocks::player_destroy::test_support::PlayerFixture,
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        api::events::{
            entity::entity_pose_change::EntityPoseChangeEvent,
            player::player_bed::PlayerBedLeaveEvent,
        },
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);

#[path = "round3_tests.rs"]
mod round3;

#[path = "position_rules_tests.rs"]
mod position_rules;

#[path = "wake_coherence_tests.rs"]
mod wake_coherence;

#[path = "logout_tests.rs"]
mod logout;

#[path = "entry_sync_tests.rs"]
mod entry_sync;

#[path = "geometry_callback_tests.rs"]
mod geometry_callback;

fn use_block(fixture: &PlayerFixture, client: &TestPlayer, position: BlockPos) {
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(0),
                position,
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.world.server.upgrade().unwrap(),
        )
        .unwrap();
}

fn sleeping(fixture: &PlayerFixture) -> TestPlayer {
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let client = TestPlayer::new(&fixture.world);
    client.player.permission_lvl.store(PermissionLvl::Four);
    client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 6.5));
    client.player.get_entity().set_rotation(-90.0, 0.0);
    client
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
    use_block(fixture, &client, FOOT.down());
    assert_eq!(fixture.world.get_block(&FOOT), &Block::WHITE_BED);
    assert_eq!(fixture.world.get_block(&HEAD), &Block::WHITE_BED);
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    use_block(fixture, &client, FOOT);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    client
}

fn break_bed(fixture: &PlayerFixture, client: &TestPlayer, position: BlockPos) {
    client.player.gamemode.store(GameMode::Creative);
    client.client().handle_player_action(
        &client.player,
        &SPlayerAction {
            status: VarInt(0),
            position,
            face: 1,
            sequence: VarInt(2),
        },
        &fixture.world.server.upgrade().unwrap(),
    );
    client.player.gamemode.store(GameMode::Survival);
    assert_eq!(fixture.world.get_block(&HEAD), &Block::AIR);
    assert_eq!(fixture.world.get_block(&FOOT), &Block::AIR);
}

fn tick(fixture: &PlayerFixture, client: &TestPlayer) {
    client.player.tick(&fixture.world.server.upgrade().unwrap());
}

fn assert_awake(client: &TestPlayer) {
    assert_eq!(
        client.player.sleeping_since.load(),
        None,
        "missing bed must wake on the next normal player tick"
    );
    assert_eq!(client.player.sleeping_bed_pos.load(), None);
    assert!(client.player.get_entity().pose.load() == EntityPose::Standing);
}

async fn finish(fixture: PlayerFixture) {
    tokio::time::timeout(std::time::Duration::from_secs(20), fixture.finish())
        .await
        .unwrap();
}

struct Callback<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

fn on_leave(
    fixture: &PlayerFixture,
    callback: impl Fn(&mut PlayerBedLeaveEvent) + Send + Sync + 'static,
) {
    fixture
        .world
        .server
        .upgrade()
        .unwrap()
        .plugin_manager
        .register::<PlayerBedLeaveEvent, _>(
            Arc::new(Callback(callback)),
            EventPriority::Normal,
            true,
        );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_intact_bed_remains_asleep_on_normal_player_tick() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    tick(&fixture, &client);
    assert_eq!(client.player.sleeping_since.load(), Some(1));
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_destroyed_bed_wakes_on_next_normal_player_tick() {
    for position in [FOOT, HEAD] {
        let fixture = PlayerFixture::new();
        let client = sleeping(&fixture);
        break_bed(&fixture, &client, position);
        assert_eq!(client.player.sleeping_since.load(), Some(0));
        tick(&fixture, &client);
        assert_awake(&client);
        assert_eq!(fixture.world.get_block(&HEAD), &Block::AIR);
        assert_eq!(fixture.world.get_block(&FOOT), &Block::AIR);
        finish(fixture).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_replaced_bed_wakes_without_overwriting_replacement() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    fixture
        .world
        .set_block_state(&HEAD, Block::CHEST.default_state.id, BlockFlags::NOTIFY_ALL);
    tick(&fixture, &client);
    assert_awake(&client);
    assert_eq!(fixture.world.get_block(&HEAD), &Block::CHEST);
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_missing_bed_leave_callback_preserves_new_bed() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    let head = fixture.world.get_block_state_id(&HEAD);
    let foot = fixture.world.get_block_state_id(&FOOT);
    break_bed(&fixture, &client, HEAD);
    let world = fixture.world.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_leave(&fixture, move |event| {
        seen.fetch_add(1, Relaxed);
        assert_eq!(event.bed_pos, HEAD);
        assert_eq!(event.player.sleeping_bed_pos.load(), Some(HEAD));
        assert_eq!(event.player.sleeping_since.load(), Some(0));
        event.player.wake_up();
        world.set_block_state(
            &FOOT,
            foot,
            BlockFlags::FORCE_STATE
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
        world.set_block_state(
            &HEAD,
            head,
            BlockFlags::FORCE_STATE
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
        assert_eq!(world.get_block_state_id(&HEAD), head);
        assert_eq!(world.get_block_state_id(&FOOT), foot);
    });
    tick(&fixture, &client);
    assert_awake(&client);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(fixture.world.get_block_state_id(&HEAD), head);
    assert_eq!(fixture.world.get_block_state_id(&FOOT), foot);
    assert!(WhiteBedLikeProperties::from_state_id(head).occupied);
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_leave_callback_new_sleep_survives_old_cleanup() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    let head = fixture.world.get_block_state_id(&HEAD);
    let foot = fixture.world.get_block_state_id(&FOOT);
    break_bed(&fixture, &client, FOOT);
    let world = fixture.world.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    on_leave(&fixture, move |event| {
        seen.fetch_add(1, Relaxed);
        world.set_block_state(
            &FOOT,
            foot,
            BlockFlags::FORCE_STATE
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
        world.set_block_state(
            &HEAD,
            head,
            BlockFlags::FORCE_STATE
                | BlockFlags::UPDATE_KNOWN_SHAPE
                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );
        assert_eq!(world.get_block_state_id(&HEAD), head);
        assert_eq!(world.get_block_state_id(&FOOT), foot);
        event.player.sleep(HEAD);
    });
    tick(&fixture, &client);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert_eq!(client.player.sleeping_since.load(), Some(1));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_pose_callback_new_sleep_survives_old_pose_commit() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    let head = fixture.world.get_block_state_id(&HEAD);
    let foot = fixture.world.get_block_state_id(&FOOT);
    break_bed(&fixture, &client, HEAD);
    let world = fixture.world.clone();
    let player = client.player.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    fixture
        .world
        .server
        .upgrade()
        .unwrap()
        .plugin_manager
        .register::<EntityPoseChangeEvent, _>(
            Arc::new(Callback(move |event: &mut EntityPoseChangeEvent| {
                if event.entity_id == player.entity_id()
                    && event.pose == (EntityPose::Standing as u8).to_string()
                {
                    seen.fetch_add(1, Relaxed);
                    world.set_block_state(
                        &FOOT,
                        foot,
                        BlockFlags::FORCE_STATE
                            | BlockFlags::UPDATE_KNOWN_SHAPE
                            | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
                    );
                    world.set_block_state(
                        &HEAD,
                        head,
                        BlockFlags::FORCE_STATE
                            | BlockFlags::UPDATE_KNOWN_SHAPE
                            | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
                    );
                    assert_eq!(world.get_block_state_id(&HEAD), head);
                    assert_eq!(world.get_block_state_id(&FOOT), foot);
                    player.sleep(HEAD);
                }
            })),
            EventPriority::Normal,
            true,
        );
    tick(&fixture, &client);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert_eq!(client.player.sleeping_since.load(), Some(1));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    finish(fixture).await;
}
