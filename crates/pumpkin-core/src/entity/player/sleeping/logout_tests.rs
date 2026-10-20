#![expect(
    clippy::unwrap_used,
    reason = "Regression scenarios require valid setup"
)]

use pumpkin_data::{
    Block, BlockDirection, block_properties::WhiteBedLikeProperties, dimension::Dimension,
    entity::EntityPose, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    GameMode, PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::data::player_data::PlayerDataStorage;
use uuid::Uuid;

use crate::{
    entity::{EntityBase, NBTStorage},
    server::scenario::{Scenario, Session, run as run_scenario},
    world::World,
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);
const NEAR_BED: Vector3<f64> = Vector3::new(5.5, 64.0, 6.5);

async fn run(body: impl Future<Output = ()>) {
    run_scenario(async {
        tokio::time::timeout(std::time::Duration::from_secs(600), body)
            .await
            .unwrap();
    })
    .await;
}

fn use_block(scenario: &Scenario, session: &Session, position: BlockPos) {
    session.send(
        &scenario.server,
        &SUseItemOn {
            hand: VarInt(0),
            position,
            face: VarInt(BlockDirection::Up as i32),
            cursor_pos: Vector3::new(0.5, 1.0, 0.5),
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(1),
        },
    );
}

fn occupied(world: &World) -> [bool; 2] {
    [FOOT, HEAD].map(|position| {
        assert_eq!(world.get_block(&position), &Block::WHITE_BED);
        WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&position)).occupied
    })
}

fn admit_sleep(scenario: &Scenario, session: &Session) {
    session.player.get_entity().set_pos(NEAR_BED);
    use_block(scenario, session, FOOT);
    assert_eq!(session.player.sleeping_bed_pos.load(), Some(HEAD));
    assert_eq!(session.player.sleeping_since.load(), Some(0));
    assert_eq!(occupied(&session.player.world()), [true, true]);
}

fn assert_awake(session: &Session) {
    assert_eq!(session.player.sleeping_bed_pos.load(), None);
    assert_eq!(session.player.sleeping_since.load(), None);
    assert!(session.player.get_entity().pose.load() == EntityPose::Standing);
}

fn serialized_timer(session: &Session) -> i16 {
    let mut nbt = NbtCompound::new();
    NBTStorage::write_nbt(session.player.as_ref(), &mut nbt);
    nbt.get_short("SleepTimer").unwrap()
}

async fn old_save_roundtrip(scenario: &Scenario, legacy: bool, timer: i16) {
    let id = Uuid::new_v4();
    let storage = PlayerDataStorage::new(scenario.directory.path().join("players"), true);
    let mut nbt = NbtCompound::new();
    if legacy {
        nbt.put_byte("playerGameType", GameMode::Survival as i8);
    } else {
        nbt.put_int("playerGameType", GameMode::Survival as i32);
    }
    nbt.put_short("SleepTimer", timer);
    nbt.put_string("Dimension", Dimension::OVERWORLD.minecraft_name.into());
    nbt.put_list("Pos", [7.5, 64.0, 7.5].map(NbtTag::Double).to_vec());
    nbt.put_float("Health", 13.0);
    let mut item = NbtCompound::new();
    item.put_byte("Slot", 4);
    ItemStack::new(17, &Item::EMERALD).write_item_stack(&mut item);
    nbt.put_list("Inventory", vec![NbtTag::Compound(item)]);
    storage
        .save_snapshot(storage.snapshot(&id, || nbt))
        .unwrap();

    let session = scenario.join(id, "SavedCounter").await;
    assert_awake(&session);
    assert_eq!(
        serialized_timer(&session),
        timer,
        "saved short counter narrowed or reset"
    );
    assert_eq!(session.player.living_entity.health.load(), 13.0);
    let stack = session.player.inventory.get_slot(4);
    assert_eq!(stack.item, &Item::EMERALD);
    assert_eq!(stack.item_count, 17);
    assert!(!session.player.world().should_skip_night());
    scenario.ticks(2).await;
    assert_awake(&session);
    assert_eq!(serialized_timer(&session), timer);
    scenario.leave(&session).await;
    let (found, saved) = storage.load_player_data(&id).unwrap();
    assert!(found);
    assert_eq!(saved.get_short("SleepTimer"), Some(timer));
}

#[tokio::test]
async fn b1020d_g01_logout_releases_bed() {
    run(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let sleeper = scenario.join(id, "Sleeper").await;
        let watcher = scenario.join(Uuid::new_v4(), "Watcher").await;
        let world = sleeper.player.world();
        sleeper.player.permission_lvl.store(PermissionLvl::Four);
        watcher.player.permission_lvl.store(PermissionLvl::Four);
        sleeper.player.get_entity().set_pos(NEAR_BED);
        sleeper.player.get_entity().set_rotation(-90.0, 0.0);
        sleeper
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
        use_block(&scenario, &sleeper, FOOT.down());
        world.set_time_of_day(13_000);

        admit_sleep(&scenario, &sleeper);
        sleeper.player.wake_up();
        assert_awake(&sleeper);
        assert_eq!(occupied(&world), [false, false], "manual wake control");
        admit_sleep(&scenario, &sleeper);
        for _ in 0..3 {
            sleeper.player.tick(&scenario.server);
        }
        assert_eq!(sleeper.player.sleeping_since.load(), Some(3));
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        scenario.leave(&sleeper).await;
        assert!(world.get_player_by_uuid(id).is_none());
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        assert_eq!(
            occupied(&world),
            [false, false],
            "#28: disconnect left the retained loaded bed occupied"
        );
        assert_awake(&sleeper);
        admit_sleep(&scenario, &watcher);
        watcher.player.wake_up();
        assert_awake(&watcher);
        assert_eq!(occupied(&world), [false, false]);

        let storage = PlayerDataStorage::new(scenario.directory.path().join("players"), true);
        let (_, saved) = storage.load_player_data(&id).unwrap();
        assert_eq!(
            saved.get_short("SleepTimer"),
            Some(3),
            "logout captures before removal"
        );
        let returning = scenario.join(id, "Sleeper").await;
        assert_awake(&returning);
        assert_eq!(serialized_timer(&returning), 3);
        assert!(!world.should_skip_night());
        scenario.leave(&returning).await;
        let ordinary = scenario.join(id, "Sleeper").await;
        assert_awake(&ordinary);
        assert_eq!(serialized_timer(&ordinary), 3, "ordinary rejoin control");
        scenario.leave(&ordinary).await;

        old_save_roundtrip(&scenario, true, 32_000).await;
        old_save_roundtrip(&scenario, false, 100).await;
        old_save_roundtrip(&scenario, false, -7).await;
        scenario.stop().await;
    })
    .await;
}

fn assert_standing_body(session: &Session) {
    use pumpkin_protocol::java::client::play::Metadata;
    let entity = session.player.get_entity();
    let standing = crate::entity::Entity::get_entity_dimensions(EntityPose::Standing);
    let actual = entity.entity_dimension.load();
    assert_eq!(
        (actual.width, actual.height, actual.eye_height),
        (standing.width, standing.height, standing.eye_height),
        "departure must retire sleeping dimensions as well as the pose flag"
    );
    let position = session.player.position();
    let expected = pumpkin_util::math::boundingbox::BoundingBox::new_from_pos(
        position.x, position.y, position.z, &standing,
    );
    let bounds = entity.bounding_box.load();
    assert_eq!((bounds.min, bounds.max), (expected.min, expected.max));
    let version = pumpkin_data::packet::CURRENT_MC_VERSION;
    let metadata = entity
        .synched_data
        .get_non_default_values_for_version(&version)
        .unwrap();
    let mut pose = Vec::new();
    Metadata::new(
        pumpkin_data::tracked_data::entity::DATA_POSE,
        VarInt(EntityPose::Standing as i32),
    )
    .write(&mut pose, &version)
    .unwrap();
    assert!(
        metadata.windows(pose.len()).any(|entry| entry == pose),
        "tracked pose must agree with forced retirement"
    );
    let mut bed = Vec::new();
    Metadata::new(
        pumpkin_data::tracked_data::player::SLEEPING_POS_ID,
        None::<BlockPos>,
    )
    .write(&mut bed, &version)
    .unwrap();
    assert!(
        metadata.windows(bed.len()).any(|entry| entry == bed),
        "tracked sleeping position must be cleared"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_logout_releases_bed_cancelled_standing() {
    use super::{Callback, EntityPoseChangeEvent, EventPriority};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    };

    run(async {
        let scenario = Scenario::new().await;
        let sleeper = scenario.join(Uuid::new_v4(), "Sleeper").await;
        let watcher = scenario.join(Uuid::new_v4(), "Watcher").await;
        let world = sleeper.player.world();
        sleeper.player.permission_lvl.store(PermissionLvl::Four);
        watcher.player.permission_lvl.store(PermissionLvl::Four);
        sleeper.player.get_entity().set_pos(NEAR_BED);
        sleeper.player.get_entity().set_rotation(-90.0, 0.0);
        sleeper
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
        use_block(&scenario, &sleeper, FOOT.down());
        world.set_time_of_day(13_000);
        admit_sleep(&scenario, &sleeper);
        for _ in 0..3 {
            sleeper.player.tick(&scenario.server);
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let player = sleeper.player.clone();
        scenario
            .server
            .plugin_manager
            .register::<EntityPoseChangeEvent, _>(
                Arc::new(Callback(move |event: &mut EntityPoseChangeEvent| {
                    if event.entity_id == player.entity_id()
                        && event.pose == (EntityPose::Standing as u8).to_string()
                    {
                        seen.fetch_add(1, Relaxed);
                        event.cancelled = true;
                    }
                })),
                EventPriority::Normal,
                true,
            );
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        scenario.leave(&sleeper).await;
        assert_eq!(calls.load(Relaxed), 1);
        assert_awake(&sleeper);
        assert_standing_body(&sleeper);
        assert_eq!(
            serialized_timer(&sleeper),
            0,
            "departed runtime retains no active sleep counter"
        );
        assert_eq!(occupied(&world), [false, false]);
        assert!(!world.should_skip_night());
        assert!(
            world
                .get_player_by_uuid(sleeper.player.gameprofile.id)
                .is_none()
        );
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        admit_sleep(&scenario, &watcher);
        watcher.player.wake_up();
        assert_awake(&watcher);
        assert_eq!(occupied(&world), [false, false]);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_logout_releases_bed_reentrant_sleep() {
    use super::{Callback, EventPriority, PlayerBedLeaveEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    };

    const OTHER_FOOT: BlockPos = BlockPos::new(5, 64, 9);
    const OTHER_HEAD: BlockPos = BlockPos::new(6, 64, 9);
    run(async {
        let scenario = Scenario::new().await;
        let sleeper = scenario.join(Uuid::new_v4(), "Sleeper").await;
        let watcher = scenario.join(Uuid::new_v4(), "BedWatcher").await;
        let world = sleeper.player.world();
        sleeper.player.permission_lvl.store(PermissionLvl::Four);
        watcher.player.permission_lvl.store(PermissionLvl::Four);
        sleeper.player.get_entity().set_pos(NEAR_BED);
        sleeper.player.get_entity().set_rotation(-90.0, 0.0);
        sleeper
            .player
            .inventory()
            .set_stack(0, ItemStack::new(2, &Item::WHITE_BED));
        use_block(&scenario, &sleeper, FOOT.down());
        use_block(&scenario, &sleeper, OTHER_FOOT.down());
        assert_eq!(world.get_block(&OTHER_HEAD), &Block::WHITE_BED);
        world.set_time_of_day(13_000);
        admit_sleep(&scenario, &sleeper);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let id = sleeper.player.gameprofile.id;
        scenario
            .server
            .plugin_manager
            .register::<PlayerBedLeaveEvent, _>(
                Arc::new(Callback(move |event: &mut PlayerBedLeaveEvent| {
                    if event.player.gameprofile.id == id {
                        seen.fetch_add(1, Relaxed);
                        event.player.sleep(HEAD);
                        event.player.sleep(OTHER_HEAD);
                    }
                })),
                EventPriority::Normal,
                true,
            );
        scenario.leave(&sleeper).await;
        assert_eq!(calls.load(Relaxed), 1);
        assert_awake(&sleeper);
        assert_standing_body(&sleeper);
        assert_eq!(serialized_timer(&sleeper), 0);
        assert_eq!(
            occupied(&world),
            [false, false],
            "reentrant public sleep cannot invalidate departure cleanup"
        );
        for position in [OTHER_FOOT, OTHER_HEAD] {
            assert_eq!(world.get_block(&position), &Block::WHITE_BED);
            assert!(
                !WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&position))
                    .occupied,
                "a retiring player cannot reserve another bed from its leave callback"
            );
        }
        assert!(world.get_player_by_uuid(id).is_none());
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        admit_sleep(&scenario, &watcher);
        watcher.player.wake_up();
        assert_awake(&watcher);
        watcher.player.get_entity().set_pos(OTHER_FOOT.to_f64());
        use_block(&scenario, &watcher, OTHER_FOOT);
        assert_eq!(watcher.player.sleeping_bed_pos.load(), Some(OTHER_HEAD));
        watcher.player.wake_up();
        assert_awake(&watcher);
        scenario.stop().await;
    })
    .await;
}

struct RemoveDuringLeave {
    player: Uuid,
    first: std::sync::atomic::AtomicBool,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::plugin::EventHandler<super::PlayerBedLeaveEvent> for RemoveDuringLeave {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a std::sync::Arc<crate::server::Server>,
        event: &'a mut super::PlayerBedLeaveEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        Box::pin(async move {
            use std::sync::atomic::Ordering::Relaxed;
            if event.player.gameprofile.id != self.player {
                return;
            }
            self.calls.fetch_add(1, Relaxed);
            // A second event records the old bug without recursively removing forever.
            if self.first.swap(false, Relaxed) {
                tokio::time::timeout(std::time::Duration::from_secs(30), event.player.remove())
                    .await
                    .unwrap();
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_logout_releases_bed_reentrant_remove() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    };

    run(async {
        let scenario = Scenario::new().await;
        let sleeper = scenario.join(Uuid::new_v4(), "RemovedInLeave").await;
        let watcher = scenario.join(Uuid::new_v4(), "Watcher").await;
        let world = sleeper.player.world();
        sleeper.player.permission_lvl.store(PermissionLvl::Four);
        watcher.player.permission_lvl.store(PermissionLvl::Four);
        sleeper.player.get_entity().set_pos(NEAR_BED);
        sleeper.player.get_entity().set_rotation(-90.0, 0.0);
        sleeper
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
        use_block(&scenario, &sleeper, FOOT.down());
        world.set_time_of_day(13_000);
        admit_sleep(&scenario, &sleeper);
        let calls = Arc::new(AtomicUsize::new(0));
        scenario
            .server
            .plugin_manager
            .register::<super::PlayerBedLeaveEvent, _>(
                Arc::new(RemoveDuringLeave {
                    player: sleeper.player.gameprofile.id,
                    first: AtomicBool::new(true),
                    calls: calls.clone(),
                }),
                super::EventPriority::Normal,
                true,
            );
        scenario.leave(&sleeper).await;
        assert_eq!(
            calls.load(Relaxed),
            1,
            "reentrant remove must not redispatch the in-flight Leave event"
        );
        assert_awake(&sleeper);
        assert_standing_body(&sleeper);
        assert_eq!(serialized_timer(&sleeper), 0);
        assert_eq!(occupied(&world), [false, false]);
        assert!(
            world
                .get_player_by_uuid(sleeper.player.gameprofile.id)
                .is_none()
        );
        assert!(world.level.is_chunk_watched(&HEAD.chunk_position()));
        admit_sleep(&scenario, &watcher);
        watcher.player.wake_up();
        assert_awake(&watcher);
        assert_eq!(occupied(&world), [false, false]);
        scenario.stop().await;
    })
    .await;
}
