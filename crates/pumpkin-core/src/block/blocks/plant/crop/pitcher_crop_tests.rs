use std::sync::Arc;

use futures::future::BoxFuture;
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{DoubleBlockHalf, PitcherCropLikeProperties},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;

use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        EventHandler, EventPriority, api::events::block::block_fertilize::BlockFertilizeEvent,
    },
    server::Server,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

const CROP: BlockPos = BlockPos::new(8, 64, 8);

async fn setup() -> (DeathTestWorld, Arc<World>, TestPlayer) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::FARMLAND));
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 6.5));
    (fixture, world, player)
}

fn put(world: &Arc<World>, pos: &BlockPos, state_id: pumpkin_data::BlockStateId) {
    world.set_block_state(
        pos,
        state_id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn plant(world: &Arc<World>, lower: BlockPos, age: u8) {
    let mut props = PitcherCropLikeProperties::default(&Block::PITCHER_CROP);
    props.age = age;
    props.half = DoubleBlockHalf::Lower;
    put(world, &lower, props.to_state_id(&Block::PITCHER_CROP));
    if age >= 3 {
        props.half = DoubleBlockHalf::Upper;
        put(world, &lower.up(), props.to_state_id(&Block::PITCHER_CROP));
    }
}

fn hold_bone_meal(player: &TestPlayer, count: u8) {
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(count, &Item::BONE_MEAL));
}

fn use_bone_meal(fixture: &DeathTestWorld, player: &TestPlayer, pos: BlockPos) {
    let result = player.client().handle_use_item_on(
        &player.player,
        &SUseItemOn {
            hand: VarInt(0),
            position: pos,
            face: VarInt(1),
            cursor_pos: Vector3::new(0.5, 1.0, 0.5),
            inside_block: false,
            is_against_world_border: false,
            sequence: VarInt(1),
        },
        &fixture.server,
    );
    assert!(result.is_ok(), "use-item-on packet should be accepted");
}

fn held_count(player: &TestPlayer) -> u8 {
    player
        .player
        .inventory()
        .get_stack_in_hand(Hand::Right)
        .item_count
}

fn lower_age(world: &World, pos: &BlockPos) -> u8 {
    PitcherCropLikeProperties::from_state_id(world.get_block_state_id(pos)).age
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_unobstructed_age_two_crop_grows_and_consumes_bone_meal() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 2);
    hold_bone_meal(&player, 3);

    use_bone_meal(&fixture, &player, CROP);

    assert_eq!(lower_age(&world, &CROP), 3);
    assert_eq!(world.get_block(&CROP.up()), &Block::PITCHER_CROP);
    assert_eq!(held_count(&player), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_solid_ceiling_rejects_bone_meal_without_changing_crop() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 2);
    put(&world, &CROP.up(), Block::DIAMOND_BLOCK.default_state.id);
    hold_bone_meal(&player, 3);

    use_bone_meal(&fixture, &player, CROP);

    assert_eq!(world.get_block(&CROP.up()), &Block::DIAMOND_BLOCK);
    assert_eq!(lower_age(&world, &CROP), 2);
    assert_eq!(held_count(&player), 3);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_upper_half_target_grows_lower_age() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 3);
    hold_bone_meal(&player, 2);

    use_bone_meal(&fixture, &player, CROP.up());

    assert_eq!(lower_age(&world, &CROP), 4);
    assert_eq!(world.get_block(&CROP.up()), &Block::PITCHER_CROP);
    assert_eq!(held_count(&player), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_max_age_upper_target_does_not_consume_bone_meal() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 4);
    hold_bone_meal(&player, 2);

    use_bone_meal(&fixture, &player, CROP.up());

    assert_eq!(lower_age(&world, &CROP), 4);
    assert_eq!(world.get_block(&CROP.up()), &Block::PITCHER_CROP);
    assert_eq!(held_count(&player), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_upper_build_height_rejects_growth() {
    let (fixture, world, player) = setup().await;
    let lower = BlockPos::new(CROP.0.x, world.get_top_y(), CROP.0.z);
    plant(&world, lower, 2);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, f64::from(world.get_top_y() - 1), 6.5));
    hold_bone_meal(&player, 2);

    use_bone_meal(&fixture, &player, lower);

    assert_eq!(lower_age(&world, &lower), 2);
    assert_ne!(world.get_block(&lower.up()), &Block::PITCHER_CROP);
    assert_eq!(held_count(&player), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_insufficient_light_rejects_growth() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 2);
    world.set_sky_light_level(&CROP, 0);
    world.set_block_light_level(&CROP, 0);
    assert_eq!(world.get_raw_brightness(&CROP, 0), 0);
    hold_bone_meal(&player, 2);

    use_bone_meal(&fixture, &player, CROP);

    assert_eq!(lower_age(&world, &CROP), 2);
    assert!(world.get_block_state(&CROP.up()).is_air());
    assert_eq!(held_count(&player), 2);
    fixture.server.shutdown().await;
}

struct FillUpperDuringFertilize {
    world: Arc<World>,
    upper: BlockPos,
}

impl EventHandler<BlockFertilizeEvent> for FillUpperDuringFertilize {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut BlockFertilizeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.world.set_block_state(
                &self.upper,
                Block::DIAMOND_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g06_fertilize_callback_obstruction_rejects_both_halves() {
    let (fixture, world, player) = setup().await;
    plant(&world, CROP, 2);
    let handler = Arc::new(FillUpperDuringFertilize {
        world: world.clone(),
        upper: CROP.up(),
    });
    fixture
        .server
        .plugin_manager
        .register::<BlockFertilizeEvent, _>(handler, EventPriority::Normal, true);
    hold_bone_meal(&player, 2);

    use_bone_meal(&fixture, &player, CROP);

    assert_eq!(world.get_block(&CROP.up()), &Block::DIAMOND_BLOCK);
    assert_eq!(lower_age(&world, &CROP), 2);
    assert_eq!(held_count(&player), 2);
    fixture.server.shutdown().await;
}
