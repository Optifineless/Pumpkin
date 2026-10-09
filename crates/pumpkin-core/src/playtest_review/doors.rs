use super::*;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::block_properties::{DoubleBlockHalf, OakDoorLikeProperties};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn powered_copper_door_transforms_both_halves_from_either_half() {
    for half in [DoubleBlockHalf::Lower, DoubleBlockHalf::Upper] {
        let fixture = Fixture::new();
        let player = TestPlayer::new(&fixture.world);
        let pos = BlockPos::new(8, 64, 9);
        fixture.world.set_block_state(
            &pos.down(),
            Block::STONE.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let mut props = OakDoorLikeProperties::default(&Block::WAXED_COPPER_DOOR);
        props.powered = true;
        props.open = true;
        for (position, half) in [
            (pos, DoubleBlockHalf::Lower),
            (pos.up(), DoubleBlockHalf::Upper),
        ] {
            props.half = half;
            fixture.world.set_block_state(
                &position,
                props.to_state_id(&Block::WAXED_COPPER_DOOR),
                BlockFlags::FORCE_STATE,
            );
        }
        // Real power prevents neighbor notifications from legitimately closing the door.
        fixture.world.set_block_state(
            &BlockPos::new(9, 64, 9),
            Block::REDSTONE_BLOCK.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        player.player.get_entity().set_sneaking(true);
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
        super::transformations::use_axe(
            &fixture,
            &player,
            if half == DoubleBlockHalf::Lower {
                pos
            } else {
                pos.up()
            },
            pumpkin_util::Hand::Right,
        );
        for (position, half) in [
            (pos, DoubleBlockHalf::Lower),
            (pos.up(), DoubleBlockHalf::Upper),
        ] {
            assert_eq!(fixture.world.get_block(&position), &Block::COPPER_DOOR);
            let props =
                OakDoorLikeProperties::from_state_id(fixture.world.get_block_state_id(&position));
            assert_eq!(props.half, half);
            assert!(props.powered && props.open);
        }
        assert_eq!(fixture.drops(&Item::COPPER_DOOR), 0);
        fixture.shutdown().await;
    }
}

fn place_oak_door(fixture: &Fixture, pos: BlockPos) {
    fixture.world.set_block_state(
        &pos.down(),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut props = OakDoorLikeProperties::default(&Block::OAK_DOOR);
    for (position, half) in [
        (pos, DoubleBlockHalf::Lower),
        (pos.up(), DoubleBlockHalf::Upper),
    ] {
        props.half = half;
        fixture.world.set_block_state(
            &position,
            props.to_state_id(&Block::OAK_DOOR),
            BlockFlags::FORCE_STATE,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_break_packets_drop_one_door_from_either_half_except_creative() {
    use pumpkin_protocol::{VarInt, java::server::play::SPlayerAction};
    for creative in [false, true] {
        for half in [DoubleBlockHalf::Lower, DoubleBlockHalf::Upper] {
            let fixture = Fixture::new();
            let player = TestPlayer::new(&fixture.world);
            player
                .player
                .permission_lvl
                .store(pumpkin_util::permission::PermissionLvl::Four);
            let pos = BlockPos::new(8, 64, 9);
            place_oak_door(&fixture, pos);
            player
                .player
                .get_entity()
                .set_pos(Vector3::new(8.5, 64.0, 8.5));
            player.player.gamemode.store(if creative {
                pumpkin_util::GameMode::Creative
            } else {
                pumpkin_util::GameMode::Survival
            });
            player
                .player
                .inventory
                .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
            player.client().handle_player_action(
                &player.player,
                &SPlayerAction {
                    status: VarInt(if creative {
                        pumpkin_protocol::java::server::play::Status::StartedDigging as i32
                    } else {
                        pumpkin_protocol::java::server::play::Status::FinishedDigging as i32
                    }),
                    position: if half == DoubleBlockHalf::Lower {
                        pos
                    } else {
                        pos.up()
                    },
                    face: pumpkin_data::BlockDirection::North as u8,
                    sequence: VarInt(1),
                },
                &fixture.server,
            );
            assert!(fixture.world.get_block_state(&pos).is_air());
            assert!(fixture.world.get_block_state(&pos.up()).is_air());
            assert_eq!(fixture.drops(&Item::OAK_DOOR), u32::from(!creative));
            fixture.shutdown().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn piston_breaking_upper_half_drops_the_lower_door_once() {
    use pumpkin_data::{
        BlockDirection,
        block_properties::{Facing, StickyPistonLikeProperties},
    };
    let fixture = Fixture::new();
    let door = BlockPos::new(8, 64, 9);
    let piston = BlockPos::new(8, 65, 8);
    place_oak_door(&fixture, door);
    let mut props = StickyPistonLikeProperties::default(&Block::PISTON);
    props.facing = Facing::South;
    fixture.world.set_block_state(
        &piston,
        props.to_state_id(&Block::PISTON),
        BlockFlags::FORCE_STATE,
    );
    fixture.world.set_block_state(
        &BlockPos::new(7, 65, 8),
        Block::REDSTONE_BLOCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert!(fixture.world.block_registry.on_synced_block_event(
        &Block::PISTON,
        &fixture.world,
        &piston,
        0,
        BlockDirection::South as u8
    ));
    assert!(fixture.world.get_block_state(&door).is_air());
    assert_eq!(fixture.drops(&Item::OAK_DOOR), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluid_replacement_of_upper_half_drops_one_door() {
    use crate::block::fluid::{flowing_trait::FlowingFluid, water::FlowingWater};
    use pumpkin_data::fluid::Fluid;
    let fixture = Fixture::new();
    let door = BlockPos::new(8, 64, 9);
    place_oak_door(&fixture, door);
    // Exercise the fluid replacement path after its spread eligibility decision.
    FlowingWater.spread_to(
        &fixture.world,
        &Fluid::FLOWING_WATER,
        &door.up(),
        Block::WATER.default_state.id,
    );
    assert!(fixture.world.get_block_state(&door).is_air());
    assert_eq!(fixture.drops(&Item::OAK_DOOR), 1);
    fixture.shutdown().await;
}
