use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::{
    Block, BlockDirection, block_properties::WhiteBedLikeProperties, dimension::Dimension,
    entity::EntityPose, item::Item,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use super::{EntityBase, FOOT, HEAD, TestPlayer, position_rules::Scenario};
use crate::block::entities::{BlockEntity, piston::PistonBlockEntity};

const PISTON: BlockPos = BlockPos::new(7, 64, 6);
const CALLBACK_POSITION: Vector3<f64> = Vector3::new(12.25, 65.0, 12.75);
const CALLBACK_YAW: f32 = 27.0;
const CALLBACK_PITCH: f32 = 19.0;

struct GeometryCallback {
    position: BlockPos,
    armed: AtomicBool,
    calls: Arc<AtomicUsize>,
    callback: Box<dyn Fn() + Send + Sync>,
}

impl BlockEntity for GeometryCallback {
    fn write_nbt(&self, _: &mut NbtCompound) {}

    fn from_nbt(_: &NbtCompound, position: BlockPos) -> Self {
        Self {
            position,
            armed: AtomicBool::new(false),
            calls: Arc::new(AtomicUsize::new(0)),
            callback: Box::new(|| {}),
        }
    }

    fn resource_location(&self) -> &'static str {
        "minecraft:piston"
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn as_any(&self) -> &dyn Any {
        if self.armed.swap(false, Relaxed) {
            self.calls.fetch_add(1, Relaxed);
            (self.callback)();
        }
        self
    }
}

#[derive(Clone, Copy, Debug)]
enum Change {
    SameWorldTeleport,
    World,
    Session,
    RemoveBed,
    ReplaceBed,
}

fn assert_callback_position(scenario: &Scenario, change: Change) {
    assert_eq!(
        scenario.client.player.position(),
        CALLBACK_POSITION,
        "{change:?}: old geometry must not overwrite callback position"
    );
    assert_eq!(
        scenario.client.player.get_entity().yaw.load(),
        CALLBACK_YAW,
        "{change:?}: old geometry must not overwrite callback yaw"
    );
    assert_eq!(
        scenario.client.player.get_entity().pitch.load(),
        CALLBACK_PITCH,
        "{change:?}: old geometry must not overwrite callback pitch"
    );
}

async fn geometry_change(change: Change) {
    let mut scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let destination = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let replacement =
        TestPlayer::with_profile(&scenario.world, scenario.client.player.gameprofile.clone());
    scenario.world.players.store(Arc::new(vec![
        scenario.client.player.clone(),
        scenario.observer.player.clone(),
    ]));
    let actor = Arc::downgrade(&scenario.client.player);
    let source = Arc::downgrade(&scenario.world);
    let target = Arc::downgrade(&destination.world);
    let new_session = Arc::downgrade(&replacement.player);
    let calls = Arc::new(AtomicUsize::new(0));
    let callback = Arc::new(GeometryCallback {
        position: PISTON,
        armed: AtomicBool::new(false),
        calls: calls.clone(),
        callback: Box::new(move || {
            let player = actor.upgrade().unwrap();
            let world = source.upgrade().unwrap();
            assert!(player.sleeping_bed_pos.load().is_none());
            assert!(player.sleeping_since.load().is_none());
            assert!(player.get_entity().pose.load() == EntityPose::Standing);
            player.get_entity().set_pos(CALLBACK_POSITION);
            player
                .get_entity()
                .set_rotation(CALLBACK_YAW, CALLBACK_PITCH);
            match change {
                Change::SameWorldTeleport => {
                    assert_eq!(
                        player.request_teleport(CALLBACK_POSITION, CALLBACK_YAW, CALLBACK_PITCH),
                        Some(CALLBACK_POSITION)
                    );
                }
                Change::World => player.get_entity().set_world(target.upgrade().unwrap()),
                Change::Session => {
                    world
                        .players
                        .store(Arc::new(vec![new_session.upgrade().unwrap()]));
                }
                Change::RemoveBed | Change::ReplaceBed => {
                    let next = if matches!(change, Change::RemoveBed) {
                        &Block::AIR
                    } else {
                        &Block::CHEST
                    };
                    world.set_block_state(
                        &HEAD,
                        next.default_state.id,
                        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
                    );
                }
            }
        }),
    });
    // This adjacent cell is visited by real dismount clearance without obstructing the exit.
    scenario.world.set_block_state(
        &PISTON,
        Block::MOVING_PISTON.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario.world.add_block_entity(callback.clone());
    assert_eq!(calls.load(Relaxed), 0);
    scenario.drain();
    callback.armed.store(true, Relaxed);

    scenario.client.player.wake_up();

    assert_eq!(
        calls.load(Relaxed),
        1,
        "real stand-up geometry must invoke the native callback"
    );
    assert_callback_position(&scenario, change);
    scenario.assert_awake();
    assert!(
        !WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&FOOT)).occupied
    );
    match change {
        Change::SameWorldTeleport => assert_eq!(scenario.world.get_block(&HEAD), &Block::WHITE_BED),
        Change::World => assert!(Arc::ptr_eq(
            &scenario.client.player.world(),
            &destination.world
        )),
        Change::Session => assert!(Arc::ptr_eq(
            &scenario
                .world
                .get_player_by_uuid(replacement.player.gameprofile.id)
                .unwrap(),
            &replacement.player,
        )),
        Change::RemoveBed => assert_eq!(scenario.world.get_block(&HEAD), &Block::AIR),
        Change::ReplaceBed => assert_eq!(scenario.world.get_block(&HEAD), &Block::CHEST),
    }
    scenario.finish().await;
    destination.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_geometry_callback() {
    for change in [
        Change::SameWorldTeleport,
        Change::World,
        Change::Session,
        Change::RemoveBed,
        Change::ReplaceBed,
    ] {
        tokio::time::timeout(Duration::from_secs(60), geometry_change(change))
            .await
            .unwrap();
    }
}

async fn changed_exit(remove_support: bool) {
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    scenario.client.player.client_loaded.store(true, Relaxed);
    let source = Arc::downgrade(&scenario.world);
    let calls = Arc::new(AtomicUsize::new(0));
    let piston = BlockPos::new(7, 65, 6);
    let callback = Arc::new(GeometryCallback {
        position: piston,
        armed: AtomicBool::new(false),
        calls: calls.clone(),
        callback: Box::new(move || {
            let world = source.upgrade().unwrap();
            // Both the first exit's support and its eye cell precede the callback in X order.
            let (position, block) = if remove_support {
                (BlockPos::new(6, 63, 6), &Block::AIR)
            } else {
                (BlockPos::new(6, 65, 6), &Block::STONE)
            };
            world.set_block_state(
                &position,
                block.default_state.id,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }),
    });
    scenario.world.set_block_state(
        &piston,
        Block::MOVING_PISTON.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario.world.add_block_entity(callback.clone());
    callback.armed.store(true, Relaxed);

    scenario.client.player.wake_up();

    assert_eq!(calls.load(Relaxed), 1);
    scenario.assert_awake();
    assert_eq!(scenario.world.get_block(&HEAD), &Block::WHITE_BED);
    assert_ne!(
        scenario.client.player.position(),
        Vector3::new(6.5, 64.0, 6.5),
        "wake must reject the exit whose already-read terrain changed"
    );
    assert!(!scenario.client.player.get_entity().is_in_wall());
    let health = scenario.client.player.living_entity.health.load();
    scenario.client.player.tick(&scenario.server);
    assert_eq!(scenario.client.player.living_entity.health.load(), health);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_changed_exit_clearance() {
    tokio::time::timeout(Duration::from_secs(60), changed_exit(false))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_changed_exit_support() {
    tokio::time::timeout(Duration::from_secs(60), changed_exit(true))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_changed_piston_exit() {
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let piston = Arc::new(PistonBlockEntity {
        position: BlockPos::new(6, 65, 6),
        pushed_block_state: Block::STONE.default_state,
        facing: BlockDirection::East,
        current_progress: 0.0.into(),
        last_progress: 0.0.into(),
        extending: true,
        source: false,
    });
    scenario.world.set_block_state(
        &piston.position,
        Block::MOVING_PISTON.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario.world.add_block_entity(piston.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_position = BlockPos::new(7, 65, 6);
    let callback = Arc::new(GeometryCallback {
        position: callback_position,
        armed: AtomicBool::new(false),
        calls: calls.clone(),
        callback: Box::new(move || piston.current_progress.store(1.0)),
    });
    scenario.world.set_block_state(
        &callback_position,
        Block::MOVING_PISTON.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario.world.add_block_entity(callback.clone());
    callback.armed.store(true, Relaxed);

    scenario.client.player.wake_up();

    assert_eq!(calls.load(Relaxed), 1);
    scenario.assert_awake();
    assert_ne!(
        scenario.client.player.position(),
        Vector3::new(6.5, 64.0, 6.5),
        "wake must reject unchanged terrain with changed piston collision progress"
    );
    scenario.finish().await;
}
