use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::{
    Block, block_properties::WhiteBedLikeProperties, dimension::Dimension, entity::EntityPose,
    item::Item,
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use super::{EntityBase, FOOT, HEAD, TestPlayer, position_rules::Scenario};
use crate::block::entities::BlockEntity;

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
