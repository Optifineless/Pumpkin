#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures require valid setup"
)]

use super::*;
use pumpkin_data::{
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::java::server::play::SUseItemOn;
use pumpkin_util::{PermissionLvl, math::vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

const BED_FOOT: BlockPos = BlockPos::new(8, 64, 8);
const SURROUND_RADIUS: i32 = 2;

struct YawOrderCase {
    facing: HorizontalFacing,
    yaw: f32,
    first_allowed: (i32, i32),
    later_allowed: (i32, i32),
}

// Literal entries from AbstractBedBlock.bedSurroundStandUpOffsets. The second
// entry must win; the seventh remains open to catch the old simplified order.
const YAW_ORDER_CASES: [YawOrderCase; 8] = [
    YawOrderCase {
        facing: HorizontalFacing::North,
        yaw: -90.0,
        first_allowed: (-1, 1),
        later_allowed: (1, 0),
    },
    YawOrderCase {
        facing: HorizontalFacing::North,
        yaw: 90.0,
        first_allowed: (1, 1),
        later_allowed: (-1, 0),
    },
    YawOrderCase {
        facing: HorizontalFacing::East,
        yaw: 0.0,
        first_allowed: (-1, -1),
        later_allowed: (0, 1),
    },
    YawOrderCase {
        facing: HorizontalFacing::East,
        yaw: 180.0,
        first_allowed: (-1, 1),
        later_allowed: (0, -1),
    },
    YawOrderCase {
        facing: HorizontalFacing::South,
        yaw: 90.0,
        first_allowed: (1, -1),
        later_allowed: (-1, 0),
    },
    YawOrderCase {
        facing: HorizontalFacing::South,
        yaw: -90.0,
        first_allowed: (-1, -1),
        later_allowed: (1, 0),
    },
    YawOrderCase {
        facing: HorizontalFacing::West,
        yaw: 180.0,
        first_allowed: (1, 1),
        later_allowed: (0, -1),
    },
    YawOrderCase {
        facing: HorizontalFacing::West,
        yaw: 0.0,
        first_allowed: (1, -1),
        later_allowed: (0, 1),
    },
];

fn set_block(world: &Arc<crate::world::World>, pos: BlockPos, block: &Block) {
    world.set_block_state(&pos, block.default_state.id, BlockFlags::FORCE_STATE);
}

fn set_state(world: &Arc<crate::world::World>, pos: BlockPos, state: pumpkin_data::BlockStateId) {
    world.set_block_state(&pos, state, BlockFlags::FORCE_STATE);
}

fn clear_bed_test_area(world: &Arc<crate::world::World>) {
    for dx in -3..=3 {
        for dz in -3..=3 {
            let x = BED_FOOT.0.x + dx;
            let z = BED_FOOT.0.z + dz;
            for y in [64, 65] {
                let pos = BlockPos::new(x, y, z);
                world.remove_block_entity(&pos);
                set_block(world, pos, &Block::AIR);
            }
            set_block(world, BlockPos::new(x, 63, z), &Block::STONE);
            set_block(world, BlockPos::new(x, 62, z), &Block::AIR);
        }
    }
}

fn bed_head(foot: BlockPos, facing: HorizontalFacing) -> BlockPos {
    let offset = facing.to_offset();
    foot.add(offset.x, 0, offset.z)
}

fn placement_yaw(facing: HorizontalFacing) -> f32 {
    match facing {
        HorizontalFacing::North => 180.0,
        HorizontalFacing::South => 0.0,
        HorizontalFacing::West => 90.0,
        HorizontalFacing::East => -90.0,
    }
}

fn use_block(fixture: &DeathTestWorld, client: &TestPlayer, pos: BlockPos) {
    client
        .client()
        .handle_use_item_on(
            &client.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(pumpkin_data::BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.server,
        )
        .unwrap();
}

fn place_and_use_bed(
    fixture: &DeathTestWorld,
    client: &TestPlayer,
    facing: HorizontalFacing,
    respawn_yaw: f32,
) -> (BlockPos, BlockPos) {
    let world = fixture.world();
    clear_bed_test_area(&world);

    let player = &client.player;
    let mut operators = fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if operators.get_entry(&player.gameprofile.id).is_none() {
        operators.ops.push(pumpkin_config::op::Op::new(
            player.gameprofile.id,
            player.gameprofile.name.clone(),
            PermissionLvl::Four,
            false,
        ));
    }
    drop(operators);
    player.permission_lvl.store(PermissionLvl::Four);
    player.set_gamemode(GameMode::Survival);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 9.5));
    player.get_entity().set_rotation(placement_yaw(facing), 0.0);
    player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
    use_block(fixture, client, BED_FOOT.down());

    let head = bed_head(BED_FOOT, facing);
    assert_eq!(world.get_block(&BED_FOOT), &Block::WHITE_BED);
    assert_eq!(world.get_block(&head), &Block::WHITE_BED);
    assert_eq!(
        WhiteBedLikeProperties::from_state_id(world.get_block_state_id(&head)).facing,
        facing
    );

    *player.respawn_point.lock().unwrap() = None;
    player.get_entity().set_rotation(respawn_yaw, 0.0);
    use_block(fixture, client, head);
    let saved = player.respawn_point.lock().unwrap().clone().unwrap();
    assert_eq!(saved.position, head);
    assert_eq!(saved.yaw, respawn_yaw);
    assert!(!saved.force);
    (head, BED_FOOT)
}

fn gate_surrounding(
    world: &Arc<crate::world::World>,
    head: BlockPos,
    foot: BlockPos,
    allowed: &[(i32, i32)],
) {
    for dx in -SURROUND_RADIUS..=SURROUND_RADIUS {
        for dz in -SURROUND_RADIUS..=SURROUND_RADIUS {
            let pos = head.add(dx, 0, dz);
            if pos == head
                || pos == foot
                || allowed
                    .iter()
                    .any(|&(open_x, open_z)| (dx, dz) == (open_x, open_z))
            {
                continue;
            }
            set_block(world, pos, &Block::STONE);
        }
    }
}

fn bed_half_state(
    block: &Block,
    part: BedPart,
    facing: HorizontalFacing,
) -> pumpkin_data::BlockStateId {
    let mut properties = WhiteBedLikeProperties::default(block);
    properties.part = part;
    properties.facing = facing;
    properties.occupied = false;
    properties.to_state_id(block)
}

async fn kill_and_enter_respawn_command(fixture: &DeathTestWorld, client: &mut TestPlayer) {
    let player = client.player.clone();
    // Match the established death fixture; exercise the production Java status packet and
    // respawn transaction without coupling geometry coverage to damage admission setup.
    player.living_entity.health.store(0.0);
    assert_eq!(
        player.living_entity.health.load(),
        0.0,
        "respawn request must start from a dead player"
    );

    client.client().handle_client_status(
        &player,
        &SClientCommand {
            action_id: VarInt(SClientCommand::PERFORM_RESPAWN),
        },
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.with_outgoing_writer(crate::server::combat_test_support::wait_for_tasks(
            &fixture.server,
        )),
    )
    .await
    .expect("Java respawn command and teleport flow should finish");
    assert!(player.living_entity.health.load() > 0.0);
}

async fn finish_fixture(fixture: DeathTestWorld) {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fixture.server.shutdown(),
    )
    .await
    .expect("test server should stop within 30 seconds");
}

fn assert_respawned_at(
    client: &TestPlayer,
    destination: &Arc<crate::world::World>,
    expected: Vector3<f64>,
    require_clear: bool,
) {
    let player = &client.player;
    let actual = player.position();
    assert!(
        (actual.x - expected.x).abs() < 1.0e-6
            && (actual.y - expected.y).abs() < 1.0e-6
            && (actual.z - expected.z).abs() < 1.0e-6,
        "respawn destination was {actual:?}, expected {expected:?}"
    );
    assert!(Arc::ptr_eq(&player.world(), destination));
    assert!(
        destination
            .players
            .load()
            .iter()
            .any(|member| Arc::ptr_eq(member, player))
    );
    assert!(
        destination
            .get_player_by_uuid(player.gameprofile.id)
            .is_some_and(|member| Arc::ptr_eq(&member, player))
    );
    assert!(!player.get_entity().is_removed());

    let bounds = player.get_entity().bounding_box.load();
    assert!((bounds.min.y - expected.y).abs() < 1.0e-6);
    let teleport = player.awaiting_teleport.lock().unwrap().unwrap();
    assert_eq!(
        teleport.1, expected,
        "respawn teleport target must own the accepted destination"
    );
    if require_clear {
        assert!(
            destination.is_space_empty(bounds),
            "respawn player box intersects block collision"
        );
    }
}

fn centered(pos: BlockPos, y_offset: f64) -> Vector3<f64> {
    pos.to_f64().add_raw(0.0, y_offset, 0.0)
}

fn set_world_spawn(fixture: &DeathTestWorld) {
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 3;
        info.spawn_y = 64;
        info.spawn_z = 3;
        info
    });
}

fn replace_worldborder(world: &crate::world::World, border: crate::world::border::Worldborder) {
    *world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = border;
}

async fn assert_anchor_respawn_control(fixture: &DeathTestWorld) {
    let nether = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    publish_nether_chunk(&nether);
    let anchor = BlockPos::new(8, 64, 8);
    let mut anchor_properties =
        RespawnAnchorLikeProperties::from_state_id(Block::RESPAWN_ANCHOR.default_state.id);
    anchor_properties.charges = 2;
    set_state(
        &nether,
        anchor,
        anchor_properties.to_state_id(&Block::RESPAWN_ANCHOR),
    );
    let mut anchor_client = TestPlayer::new(&nether);
    anchor_client
        .player
        .permission_lvl
        .store(PermissionLvl::Four);
    *anchor_client.player.respawn_point.lock().unwrap() = Some(RespawnPoint {
        dimension: Dimension::THE_NETHER,
        position: anchor,
        yaw: 0.0,
        force: false,
    });
    kill_and_enter_respawn_command(fixture, &mut anchor_client).await;
    let anchor_spawn = Vector3::new(8.5, 64.1, 7.5);
    assert_respawned_at(&anchor_client, &nether, anchor_spawn, true);
    assert_eq!(
        RespawnAnchorLikeProperties::from_state_id(nether.get_block_state_id(&anchor)).charges,
        1,
        "a successful anchor respawn consumes one charge"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g01_enclosed_bed_respawns_on_its_collision_surface() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    set_world_spawn(&fixture);
    world.level_time.lock().unwrap().time_of_day = 13_000;
    let mut client = TestPlayer::new(&world);
    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 0.0);

    let open_candidate = head.add(1, 0, 0);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    let open_position = client.player.position();
    assert_eq!(open_position.x, f64::from(open_candidate.0.x) + 0.5);
    assert_eq!(open_position.z, f64::from(open_candidate.0.z) + 0.5);
    assert!(world.is_space_empty(client.player.get_entity().bounding_box.load()));

    gate_surrounding(&world, head, foot, &[]);
    assert_eq!(world.get_block(&head.up()), &Block::AIR);
    assert_eq!(world.get_block(&foot.up()), &Block::AIR);
    kill_and_enter_respawn_command(&fixture, &mut client).await;

    let bed_surface = centered(head, 0.5625);
    assert_respawned_at(&client, &world, bed_surface, true);
    finish_fixture(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g01_bed_head_foot_and_yaw_candidate_order() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    set_world_spawn(&fixture);
    world.level_time.lock().unwrap().time_of_day = 13_000;
    let mut client = TestPlayer::new(&world);

    for case in YAW_ORDER_CASES {
        let (head, foot) = place_and_use_bed(&fixture, &client, case.facing, case.yaw);
        gate_surrounding(
            &world,
            head,
            foot,
            &[case.first_allowed, case.later_allowed],
        );
        let expected_block = head.add(case.first_allowed.0, 0, case.first_allowed.1);
        kill_and_enter_respawn_command(&fixture, &mut client).await;
        assert_respawned_at(&client, &world, centered(expected_block, 0.0), true);
    }

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    gate_surrounding(&world, head, foot, &[]);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(head, 0.5625), true);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, -90.0);
    gate_surrounding(&world, head, foot, &[]);
    set_block(&world, head.up(), &Block::STONE);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(foot, 0.5625), true);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    let lower_head = head.down();
    let lower_foot = foot.down();
    set_state(
        &world,
        lower_head,
        bed_half_state(&Block::WHITE_BED, BedPart::Head, HorizontalFacing::North),
    );
    set_state(
        &world,
        lower_foot,
        bed_half_state(&Block::WHITE_BED, BedPart::Foot, HorizontalFacing::North),
    );
    for dx in -SURROUND_RADIUS..=SURROUND_RADIUS {
        for dz in -SURROUND_RADIUS..=SURROUND_RADIUS {
            set_block(&world, head.add(dx, -2, dz), &Block::STONE);
        }
    }
    let lower_first = head.add(1, -1, 0);
    set_block(&world, lower_first, &Block::AIR);
    // Leave headroom above the lower first candidate clear. The upper candidate at
    // that horizontal cell has no support and must be skipped before the bunk pass.
    gate_surrounding(&world, head, foot, &[(1, 0)]);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(lower_first, 0.0), true);

    finish_fixture(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g01_straw_bed_below_counts_as_bunk_support() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    set_world_spawn(&fixture);
    world.level_time.lock().unwrap().time_of_day = 13_000;
    let mut client = TestPlayer::new(&world);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    let lower_head = head.down();
    let lower_foot = foot.down();
    set_state(
        &world,
        lower_head,
        bed_half_state(&Block::STRAW_BED, BedPart::Head, HorizontalFacing::North),
    );
    set_state(
        &world,
        lower_foot,
        bed_half_state(&Block::STRAW_BED, BedPart::Foot, HorizontalFacing::North),
    );
    for dx in -SURROUND_RADIUS..=SURROUND_RADIUS {
        for dz in -SURROUND_RADIUS..=SURROUND_RADIUS {
            set_block(&world, head.add(dx, -2, dz), &Block::STONE);
        }
    }
    let lower_first = head.add(1, -1, 0);
    set_block(&world, lower_first, &Block::AIR);
    // The upper exit is clear horizontally but lacks a floor, so only the lower
    // AbstractBedBlock candidate should win before the bed-surface fallback.
    gate_surrounding(&world, head, foot, &[(1, 0)]);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(lower_first, 0.0), true);

    finish_fixture(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g01_respawn_geometry_rejects_collisions_and_preserves_other_spawns() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    set_world_spawn(&fixture);
    assert_anchor_respawn_control(&fixture).await;

    let mut client = TestPlayer::new(&world);
    client.player.permission_lvl.store(PermissionLvl::Four);
    let forced_pos = BlockPos::new(8, 64, 8);
    *client.player.respawn_point.lock().unwrap() = Some(RespawnPoint {
        dimension: Dimension::OVERWORLD,
        position: forced_pos,
        yaw: 45.0,
        force: true,
    });
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(forced_pos, 0.1), true);
    assert!(
        client
            .player
            .respawn_point
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .force
    );

    world.level_time.lock().unwrap().time_of_day = 13_000;
    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    gate_surrounding(&world, head, foot, &[(1, 0), (1, 1)]);
    let blocked = head.add(1, 0, 0);
    set_block(&world, blocked.up(), &Block::STONE);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(head.add(1, 0, 1), 0.0), true);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    gate_surrounding(&world, head, foot, &[(1, 0), (1, 1)]);
    set_block(&world, head.add(1, 0, 0), &Block::END_PORTAL);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(head.add(1, 0, 1), 0.0), true);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    gate_surrounding(&world, head, foot, &[(1, 0)]);
    set_block(&world, head.add(1, 0, 0), &Block::STONE_SLAB);
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    assert_respawned_at(&client, &world, centered(head.add(1, 0, 0), 0.5), true);

    let (head, foot) = place_and_use_bed(&fixture, &client, HorizontalFacing::North, 90.0);
    gate_surrounding(&world, head, foot, &[(1, 0)]);
    let border_candidate = centered(head.add(1, 0, 0), 0.0);
    let original_border = {
        let border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (*border).clone()
    };
    replace_worldborder(
        &world,
        crate::world::border::Worldborder::new(
            border_candidate.x - 0.05,
            border_candidate.z,
            0.5,
            0,
            5,
            15,
        ),
    );
    kill_and_enter_respawn_command(&fixture, &mut client).await;
    replace_worldborder(&world, original_border);
    assert_respawned_at(&client, &world, Vector3::new(3.5, 64.0, 3.5), true);
    assert_ne!(client.player.position(), border_candidate);

    finish_fixture(fixture).await;
}
