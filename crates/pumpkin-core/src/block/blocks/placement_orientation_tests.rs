use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{
        DoorHinge, DoubleBlockHalf, Half, HorizontalFacing, OakDoorLikeProperties,
        OakTrapdoorLikeProperties, WaterLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

struct PlacementFixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl PlacementFixture {
    fn new(yaw: f32) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 12.5));
        player.player.get_entity().set_rotation(yaw, 0.0);

        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn set_block(&self, position: BlockPos, block: &Block) {
        self.world.set_block_state(
            &position,
            block.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn give(&self, item: &'static Item) {
        self.player
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, item));
    }

    fn give_count(&self, item: &'static Item, count: u8) {
        self.player
            .player
            .inventory()
            .set_stack(0, ItemStack::new(count, item));
    }

    fn set_state(&self, position: BlockPos, state: pumpkin_data::BlockStateId) {
        self.world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn clear_door_volume(&self, lower: BlockPos) {
        for x in (lower.0.x - 1)..=(lower.0.x + 1) {
            for z in (lower.0.z - 1)..=(lower.0.z + 1) {
                for y in lower.0.y..=(lower.0.y + 1) {
                    self.set_block(BlockPos::new(x, y, z), &Block::AIR);
                }
            }
        }
        self.set_block(lower.down(), &Block::STONE);
    }

    fn use_item_on(&self, position: BlockPos, face: BlockDirection, cursor: Vector3<f32>) {
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(face as i32),
                    cursor_pos: cursor,
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    fn clear_door_area(&self, lower: BlockPos) {
        for x in (lower.0.x - 1)..=(lower.0.x + 1) {
            for y in lower.0.y..=(lower.0.y + 1) {
                self.set_block(BlockPos::new(x, y, lower.0.z), &Block::AIR);
            }
        }
        self.set_block(lower.down(), &Block::STONE);
    }

    fn place_door(&self, lower: BlockPos, click_x: f32) {
        self.give(&Item::OAK_DOOR);
        self.use_item_on(
            lower.down(),
            BlockDirection::Up,
            Vector3::new(click_x, 0.5, 0.5),
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn door_hinge_placement_uses_upper_obstructions_and_preserves_controls() {
    let fixture = PlacementFixture::new(180.0);
    let lower = BlockPos::new(8, 64, 8);
    let cases = [
        (
            BlockPos::new(7, 65, 8),
            0.75,
            DoorHinge::Left,
            "upper-left obstruction",
        ),
        (
            BlockPos::new(9, 65, 8),
            0.25,
            DoorHinge::Right,
            "upper-right obstruction",
        ),
        (
            BlockPos::new(7, 64, 8),
            0.5,
            DoorHinge::Left,
            "lower-left obstruction",
        ),
        (
            BlockPos::new(9, 64, 8),
            0.5,
            DoorHinge::Right,
            "lower-right obstruction",
        ),
    ];

    for (obstruction, click_x, expected_hinge, label) in cases {
        fixture.clear_door_area(lower);
        fixture.set_block(obstruction, &Block::STONE);
        fixture.place_door(lower, click_x);

        assert_eq!(fixture.world.get_block(&lower), &Block::OAK_DOOR, "{label}");
        assert_eq!(
            fixture.world.get_block(&lower.up()),
            &Block::OAK_DOOR,
            "{label} upper half"
        );
        let lower_state =
            OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
        let upper_state =
            OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower.up()));
        assert_eq!(lower_state.half, DoubleBlockHalf::Lower, "{label}");
        assert_eq!(upper_state.half, DoubleBlockHalf::Upper, "{label}");
        assert_eq!(lower_state.hinge, expected_hinge, "{label}");
        assert_eq!(upper_state.hinge, expected_hinge, "{label} upper half");
    }

    for (click_x, expected_hinge) in [(0.25, DoorHinge::Left), (0.75, DoorHinge::Right)] {
        fixture.clear_door_area(lower);
        fixture.place_door(lower, click_x);
        let state = OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
        assert_eq!(
            state.hinge, expected_hinge,
            "empty-neighbor click tie-break"
        );
    }

    fixture.clear_door_area(lower);
    fixture.place_door(lower, 0.5);
    let adjacent_lower = BlockPos::new(lower.0.x + 1, lower.0.y, lower.0.z);
    fixture.place_door(adjacent_lower, 0.25);
    let adjacent_state =
        OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&adjacent_lower));
    assert_eq!(
        adjacent_state.hinge,
        DoorHinge::Right,
        "the existing lower door takes precedence over the click tie-break"
    );

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g06_lantern_uses_context_and_flowing_water() {
    let fixture = PlacementFixture::new(180.0);
    let position = BlockPos::new(8, 64, 8);
    let clicked = position.west();
    let place = |fixture: &PlacementFixture, block: &'static Block, item: &'static Item| {
        fixture.give_count(item, 2);
        fixture.use_item_on(clicked, BlockDirection::East, Vector3::new(1.0, 0.5, 0.5));
        let state = pumpkin_data::block_properties::LanternLikeProperties::from_state_id(
            fixture.world.get_block_state_id(&position),
        );
        assert_eq!(fixture.world.get_block(&position), block);
        assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 1);
        state
    };

    let prepare = |fixture: &PlacementFixture, target: &'static Block| {
        fixture.set_block(clicked, &Block::STONE);
        fixture.set_block(position.down(), &Block::STONE);
        fixture.set_block(position, target);
        fixture.set_block(position.up(), &Block::STONE);
        fixture.player.player.get_entity().set_rotation(180.0, 75.0);
    };

    prepare(&fixture, &Block::AIR);
    let standing = place(&fixture, &Block::LANTERN, &Item::LANTERN);
    assert!(
        !standing.r#hanging,
        "looking down with both supports must stand"
    );
    assert!(!standing.r#waterlogged, "dry lantern must remain dry");

    prepare(&fixture, &Block::WATER);
    fixture
        .player
        .player
        .get_entity()
        .set_rotation(180.0, -75.0);
    let hanging = place(&fixture, &Block::SOUL_LANTERN, &Item::SOUL_LANTERN);
    assert!(hanging.r#hanging, "looking up with both supports must hang");
    assert!(
        hanging.r#waterlogged,
        "source water must waterlog the lantern"
    );

    let mut flowing_water = WaterLikeProperties::default(&Block::WATER);
    flowing_water.level = 1;
    prepare(&fixture, &Block::AIR);
    fixture.set_state(position, flowing_water.to_state_id(&Block::WATER));
    let flowing = place(&fixture, &Block::LANTERN, &Item::LANTERN);
    assert!(
        flowing.r#waterlogged,
        "flowing water must waterlog the lantern"
    );

    prepare(&fixture, &Block::AIR);
    fixture
        .player
        .player
        .get_entity()
        .set_rotation(180.0, -75.0);
    fixture.player.player.gamemode.store(GameMode::Creative);
    fixture.give_count(&Item::SOUL_LANTERN, 2);
    fixture.use_item_on(clicked, BlockDirection::East, Vector3::new(1.0, 0.5, 0.5));
    let creative = pumpkin_data::block_properties::LanternLikeProperties::from_state_id(
        fixture.world.get_block_state_id(&position),
    );
    assert!(creative.r#hanging);
    assert_eq!(fixture.world.get_block(&position), &Block::SOUL_LANTERN);
    assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 2);

    fixture.player.player.gamemode.store(GameMode::Survival);
    prepare(&fixture, &Block::AIR);
    fixture.set_block(position.down(), &Block::AIR);
    let fallback = place(&fixture, &Block::LANTERN, &Item::LANTERN);
    assert!(
        fallback.r#hanging,
        "unsupported preferred floor must use the ceiling"
    );

    prepare(&fixture, &Block::AIR);
    fixture.set_block(position.down(), &Block::AIR);
    fixture.set_block(position.up(), &Block::AIR);
    fixture.give_count(&Item::LANTERN, 2);
    fixture.use_item_on(clicked, BlockDirection::East, Vector3::new(1.0, 0.5, 0.5));
    assert!(fixture.world.get_block_state(&position).is_air());
    assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 2);

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g06_door_hinge_midpoint_is_inclusive() {
    let fixture = PlacementFixture::new(180.0);
    let lower = BlockPos::new(8, 64, 8);
    let cases = [
        (
            180.0,
            HorizontalFacing::North,
            true,
            DoorHinge::Left,
            DoorHinge::Right,
        ),
        (
            0.0,
            HorizontalFacing::South,
            true,
            DoorHinge::Right,
            DoorHinge::Left,
        ),
        (
            270.0,
            HorizontalFacing::East,
            false,
            DoorHinge::Left,
            DoorHinge::Right,
        ),
        (
            90.0,
            HorizontalFacing::West,
            false,
            DoorHinge::Right,
            DoorHinge::Left,
        ),
    ];

    for (yaw, facing, tie_on_x, below_midpoint, above_midpoint) in cases {
        fixture.player.player.get_entity().set_rotation(yaw, 0.0);
        for (value, expected) in [
            (0.5, DoorHinge::Left),
            (0.499, below_midpoint),
            (0.501, above_midpoint),
        ] {
            fixture.clear_door_volume(lower);
            fixture.give_count(&Item::OAK_DOOR, 2);
            let (x, z) = if tie_on_x { (value, 0.5) } else { (0.5, value) };
            fixture.use_item_on(lower.down(), BlockDirection::Up, Vector3::new(x, 1.0, z));

            assert_eq!(fixture.world.get_block(&lower), &Block::OAK_DOOR);
            assert_eq!(fixture.world.get_block(&lower.up()), &Block::OAK_DOOR);
            let lower_state =
                OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
            let upper_state =
                OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower.up()));
            assert_eq!(lower_state.facing, facing);
            assert_eq!(lower_state.half, DoubleBlockHalf::Lower);
            assert_eq!(upper_state.half, DoubleBlockHalf::Upper);
            assert_eq!(lower_state.hinge, expected, "{facing:?} hit {value}");
            assert_eq!(upper_state.hinge, expected, "upper {facing:?} hit {value}");
            assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 1);
        }
    }

    fixture.player.player.get_entity().set_rotation(180.0, 0.0);
    for (side, y_offset, expected) in [
        (BlockDirection::West, 0, DoorHinge::Left),
        (BlockDirection::West, 1, DoorHinge::Left),
        (BlockDirection::East, 0, DoorHinge::Right),
        (BlockDirection::East, 1, DoorHinge::Right),
    ] {
        fixture.clear_door_volume(lower);
        fixture.set_block(
            BlockPos::new(
                lower.0.x + side.to_offset().x,
                lower.0.y + y_offset,
                lower.0.z + side.to_offset().z,
            ),
            &Block::STONE,
        );
        fixture.place_door(lower, 0.5);
        let state = OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
        assert_eq!(
            state.hinge, expected,
            "obstruction {side:?} at y+{y_offset}"
        );
    }

    fixture.clear_door_volume(lower);
    fixture.place_door(lower, 0.5);
    let adjacent = lower.offset(HorizontalFacing::North.rotate_clockwise().to_offset());
    fixture.place_door(adjacent, 0.5);
    let adjacent_state =
        OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&adjacent));
    assert_eq!(adjacent_state.hinge, DoorHinge::Right);

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g06_door_hinge_uses_placement_relative_hit() {
    let fixture = PlacementFixture::new(270.0);
    let lower = BlockPos::new(8, 64, 8);
    fixture.clear_door_volume(lower);
    fixture.set_block(lower.north(), &Block::GLASS_PANE);
    fixture.give_count(&Item::OAK_DOOR, 2);
    fixture.use_item_on(
        lower.north(),
        BlockDirection::South,
        Vector3::new(0.5, 0.5, 1.0),
    );
    let state = OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
    let upper_state =
        OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower.up()));
    assert_eq!(fixture.world.get_block(&lower), &Block::OAK_DOOR);
    assert_eq!(fixture.world.get_block(&lower.up()), &Block::OAK_DOOR);
    assert_eq!(state.facing, HorizontalFacing::East);
    assert_eq!(state.half, DoubleBlockHalf::Lower);
    assert_eq!(
        state.hinge,
        DoorHinge::Left,
        "hit must be relative to the placement cell"
    );
    assert_eq!(upper_state.half, DoubleBlockHalf::Upper);
    assert_eq!(upper_state.hinge, DoorHinge::Left);
    assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 1);

    fixture.clear_door_volume(lower);
    fixture.player.player.get_entity().set_rotation(180.0, 0.0);
    fixture.set_block(lower.up(), &Block::STONE);
    fixture.give_count(&Item::OAK_DOOR, 2);
    fixture.use_item_on(
        lower.down(),
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
    );
    assert!(fixture.world.get_block_state(&lower).is_air());
    assert_eq!(fixture.player.player.inventory().get_stack(0).item_count, 2);

    fixture.clear_door_volume(lower);
    fixture.set_block(lower.north(), &Block::REDSTONE_BLOCK);
    fixture.give_count(&Item::OAK_DOOR, 2);
    fixture.use_item_on(
        lower.down(),
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
    );
    let powered = OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower));
    let powered_upper =
        OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&lower.up()));
    assert!(powered.powered && powered.open, "powered placement state");
    assert!(
        powered_upper.powered && powered_upper.open,
        "powered upper state"
    );

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trapdoor_side_placement_uses_clicked_face_and_preserves_other_branches() {
    let fixture = PlacementFixture::new(0.0);
    let clicked = BlockPos::new(8, 64, 8);
    fixture.set_block(clicked, &Block::STONE);

    let sides = [
        (
            BlockDirection::North,
            HorizontalFacing::North,
            0.25,
            Half::Bottom,
        ),
        (
            BlockDirection::East,
            HorizontalFacing::East,
            0.75,
            Half::Top,
        ),
        (
            BlockDirection::South,
            HorizontalFacing::South,
            0.25,
            Half::Bottom,
        ),
        (
            BlockDirection::West,
            HorizontalFacing::West,
            0.75,
            Half::Top,
        ),
    ];
    for (face, expected_facing, click_y, expected_half) in sides {
        let position = BlockPos::new(
            clicked.0.x + face.to_offset().x,
            clicked.0.y + face.to_offset().y,
            clicked.0.z + face.to_offset().z,
        );
        fixture.give(&Item::OAK_TRAPDOOR);
        fixture.use_item_on(clicked, face, Vector3::new(0.5, click_y, 0.5));

        assert_eq!(fixture.world.get_block(&position), &Block::OAK_TRAPDOOR);
        let state =
            OakTrapdoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&position));
        assert_eq!(state.facing, expected_facing, "clicked face {face:?}");
        assert_eq!(state.half, expected_half, "clicked face {face:?}");
    }

    for (clicked, face, expected_half) in [
        (BlockPos::new(11, 64, 11), BlockDirection::Up, Half::Bottom),
        (BlockPos::new(11, 65, 8), BlockDirection::Down, Half::Top),
    ] {
        fixture.set_block(clicked, &Block::STONE);
        fixture.give(&Item::OAK_TRAPDOOR);
        fixture.use_item_on(clicked, face, Vector3::new(0.5, 0.25, 0.5));

        let position = BlockPos::new(
            clicked.0.x + face.to_offset().x,
            clicked.0.y + face.to_offset().y,
            clicked.0.z + face.to_offset().z,
        );
        assert_eq!(fixture.world.get_block(&position), &Block::OAK_TRAPDOOR);
        let state =
            OakTrapdoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&position));
        assert_eq!(
            state.facing,
            HorizontalFacing::North,
            "vertical face {face:?}"
        );
        assert_eq!(state.half, expected_half, "vertical face {face:?}");
    }

    let water = BlockPos::new(5, 64, 9);
    fixture.set_block(water, &Block::WATER);
    fixture.give(&Item::OAK_TRAPDOOR);
    fixture.use_item_on(water, BlockDirection::East, Vector3::new(0.5, 0.75, 0.5));
    assert_eq!(fixture.world.get_block(&water), &Block::OAK_TRAPDOOR);
    let state = OakTrapdoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&water));
    assert_eq!(state.facing, HorizontalFacing::North);
    assert_eq!(state.half, Half::Top);
    assert!(state.waterlogged);
    assert!(!state.powered);
    assert!(!state.open);

    fixture.server.shutdown().await;
}
