use crate::{
    block::OnScheduledTickArgs,
    entity::death_test_world::DeathTestWorld,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block, BlockStateId, biome::Biome, block_properties::LanternLikeProperties, item::Item,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use rustc_hash::FxHashSet;
use std::sync::Arc;

fn put(world: &Arc<World>, position: BlockPos, state: BlockStateId) {
    world.set_block_state(
        &position,
        state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn item_count(world: &World, item: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity.get_item_stack().lock().unwrap();
            if stack.item == item {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

fn run_registered_block_ticks(fixture: &DeathTestWorld) {
    let world = fixture.world();
    let game_time = {
        let mut time = world.level_time.lock().unwrap();
        time.tick(world.level_info.load().game_rules.advance_time);
        time.world_age
    };
    world.level.set_game_time(game_time);
    let active_chunks = FxHashSet::from_iter([Vector2::new(0, 0)]);
    let tick_data = world.level.get_tick_data(&active_chunks, 0);
    for tick in tick_data.block_ticks {
        let block = world.get_block(&tick.position);
        if block == tick.value {
            world
                .block_registry
                .get_pumpkin_block(block.id)
                .unwrap()
                .on_scheduled_tick(OnScheduledTickArgs {
                    world: &world,
                    block,
                    position: &tick.position,
                });
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lantern_survival_uses_stored_hanging_support() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let behavior = world
        .block_registry
        .get_pumpkin_block(Block::LANTERN.id)
        .unwrap();

    let hanging_pos = BlockPos::new(8, 64, 8);
    let hanging_ceiling = hanging_pos.up();
    let hanging_floor = hanging_pos.down();
    put(&world, hanging_ceiling, Block::STONE.default_state.id);
    put(&world, hanging_floor, Block::STONE.default_state.id);
    let mut hanging = LanternLikeProperties::default(&Block::LANTERN);
    hanging.r#hanging = true;
    let hanging_state = hanging.to_state_id(&Block::LANTERN);
    put(&world, hanging_pos, hanging_state);

    behavior.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::LANTERN,
        position: &hanging_pos,
    });
    assert_eq!(world.get_block_state_id(&hanging_pos), hanging_state);
    assert_eq!(item_count(&world, &Item::LANTERN), 0);

    assert!(
        world
            .break_block(
                &hanging_ceiling,
                None,
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
            )
            .is_some()
    );
    assert_eq!(world.get_block_state_id(&hanging_pos), hanging_state);
    assert!(world.is_block_tick_scheduled(&hanging_pos, &Block::LANTERN));
    run_registered_block_ticks(&fixture);
    assert!(world.get_block_state(&hanging_pos).is_air());
    assert_eq!(item_count(&world, &Item::LANTERN), 1);

    let standing_pos = BlockPos::new(12, 64, 8);
    let standing_floor = standing_pos.down();
    let standing_ceiling = standing_pos.up();
    put(&world, standing_floor, Block::STONE.default_state.id);
    put(&world, standing_ceiling, Block::STONE.default_state.id);
    let mut standing = LanternLikeProperties::default(&Block::LANTERN);
    standing.r#hanging = false;
    let standing_state = standing.to_state_id(&Block::LANTERN);
    put(&world, standing_pos, standing_state);

    behavior.on_scheduled_tick(OnScheduledTickArgs {
        world: &world,
        block: &Block::LANTERN,
        position: &standing_pos,
    });
    assert_eq!(world.get_block_state_id(&standing_pos), standing_state);

    assert!(
        world
            .break_block(
                &standing_floor,
                None,
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
            )
            .is_some()
    );
    assert_eq!(world.get_block_state_id(&standing_pos), standing_state);
    assert!(world.is_block_tick_scheduled(&standing_pos, &Block::LANTERN));
    run_registered_block_ticks(&fixture);
    assert!(world.get_block_state(&standing_pos).is_air());
    assert_eq!(item_count(&world, &Item::LANTERN), 2);

    fixture.server.shutdown().await;
}
