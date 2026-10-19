#![expect(clippy::unwrap_used, reason = "Real End transfer safety regressions")]

use std::sync::Arc;

use pumpkin_data::{Block, dimension::Dimension};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use super::{
    end_return_test_support::{Fixture, PERSONAL, load_personal, transfer},
    end_return_tests::{assert_anchor, install_anchor, install_bed},
};
use crate::{entity::EntityBase, world::World};

fn set(world: &Arc<World>, pos: BlockPos, block: &Block) {
    world.set_block_state(&pos, block.default_state.id, BlockFlags::FORCE_STATE);
}

async fn anchor_hazard_return(hazard: &Block) {
    let fixture = Fixture::new().await;
    install_anchor(&fixture.nether);
    let north = PERSONAL.add(0, 0, -1);
    if hazard == &Block::LAVA {
        set(&fixture.nether, north, hazard);
    } else {
        set(&fixture.nether, north.down(), hazard);
    }
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::THE_NETHER, false);
    let health = player.living_entity.health.load();
    transfer(&fixture, &mut client).await;
    assert!(Arc::ptr_eq(&player.world(), &fixture.nether));
    assert_eq!(
        player.position(),
        Vector3::new(7.5, 64.0, 8.5),
        "{hazard:?}: dry west candidate must beat the dangerous first candidate"
    );
    assert_eq!(player.get_entity().yaw.load(), -90.0);
    player.get_entity().tick(player.as_ref(), &fixture.server);
    assert_eq!(player.living_entity.health.load(), health);
    assert!(!player.get_entity().is_on_fire());
    assert_anchor(&fixture.nether);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_anchor_avoids_lava_without_damage() {
    anchor_hazard_return(&Block::LAVA).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_anchor_avoids_magma_floor_without_damage() {
    anchor_hazard_return(&Block::MAGMA_BLOCK).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_anchor_avoids_campfire_floor_without_damage() {
    anchor_hazard_return(&Block::CAMPFIRE).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_bed_prefers_dry_alternative_to_lava() {
    let fixture = Fixture::new().await;
    install_bed(&fixture.overworld);
    set(&fixture.overworld, PERSONAL.add(-1, 0, 0), &Block::LAVA);
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::OVERWORLD, false);
    player.respawn_point.lock().unwrap().as_mut().unwrap().yaw = -90.0;
    transfer(&fixture, &mut client).await;
    assert_eq!(
        player.position(),
        Vector3::new(7.5, 64.0, 7.5),
        "the dry supported candidate behind the west lava cell must win"
    );
    assert!(!player.get_entity().is_on_fire());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_rejects_wall_sign_floor_over_shaft() {
    let fixture = Fixture::new().await;
    install_bed(&fixture.overworld);
    let west = PERSONAL.add(-1, 0, 0);
    let east = PERSONAL.add(1, 0, 0);
    for x in 6..=10 {
        for z in 6..=9 {
            let pos = BlockPos::new(x, 64, z);
            if pos != west && pos != east && pos != PERSONAL && pos != PERSONAL.add(0, 0, -1) {
                set(&fixture.overworld, pos, &Block::STONE);
                set(&fixture.overworld, pos.up(), &Block::STONE);
            }
        }
    }
    for y in 33..=63 {
        set(&fixture.overworld, BlockPos::new(7, y, 8), &Block::AIR);
    }
    set(&fixture.overworld, BlockPos::new(7, 32, 8), &Block::STONE);
    let sign = Block::OAK_WALL_SIGN
        .from_properties(&[("facing", "west"), ("waterlogged", "false")])
        .to_state_id(&Block::OAK_WALL_SIGN);
    fixture
        .overworld
        .set_block_state(&west.down(), sign, BlockFlags::FORCE_STATE);
    assert_eq!(fixture.overworld.get_block(&PERSONAL.down()), &Block::STONE);
    assert!(fixture.overworld.get_block_state(&west.down()).is_solid());
    assert!(fixture.overworld.get_dismount_height(&west).is_infinite());
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::OVERWORLD, false);
    player.respawn_point.lock().unwrap().as_mut().unwrap().yaw = -90.0;
    let before = player.respawn_point.lock().unwrap().clone();
    transfer(&fixture, &mut client).await;
    assert_eq!(
        player.position(),
        Vector3::new(9.5, 64.0, 8.5),
        "a legacy-solid wall sign above a deep shaft is not a floor"
    );
    let floor = fixture.overworld.get_dismount_height(&east);
    assert!(floor.is_finite() && floor < 1.0);
    assert_eq!(player.position().y, f64::from(east.0.y) + floor);
    assert_eq!(*player.respawn_point.lock().unwrap(), before);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_end_return_bed_at_build_height_keeps_air_above() {
    let fixture = Fixture::new().await;
    install_bed(&fixture.overworld);
    let top = fixture.overworld.min_y + fixture.overworld.dimension.height - 1;
    let head = BlockPos::new(8, top, 8);
    for pos in [head, head.add(0, 0, -1)] {
        let original = BlockPos::new(pos.0.x, 64, pos.0.z);
        fixture.overworld.set_block_state(
            &pos,
            fixture.overworld.get_block_state_id(&original),
            BlockFlags::FORCE_STATE,
        );
        set(&fixture.overworld, pos.down(), &Block::STONE);
    }
    set(&fixture.overworld, head.add(-1, -1, 0), &Block::STONE);
    let mut client = fixture.returning_player();
    let player = client.player.clone();
    load_personal(&player, &Dimension::OVERWORLD, false);
    player
        .respawn_point
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .position = head;
    player.respawn_point.lock().unwrap().as_mut().unwrap().yaw = -90.0;
    transfer(&fixture, &mut client).await;
    assert_eq!(
        player.position(),
        Vector3::new(7.5, f64::from(top), 8.5),
        "outside-height air must not invalidate a supported top-height bed"
    );
    fixture.finish().await;
}
