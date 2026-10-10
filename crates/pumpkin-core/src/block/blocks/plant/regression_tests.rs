use crate::{
    block::{BlockBehaviour, RandomTickArgs},
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    biome::Biome,
    block_properties::{
        CactusLikeProperties, CandleLikeProperties, RedstoneOreLikeProperties,
        SeaPickleLikeProperties, WheatLikeProperties,
    },
    fluid::Fluid,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{
    VarInt,
    java::{
        client::play::{CParticle, CSoundEffect},
        server::play::SUseItemOn,
    },
    packet::MultiVersionJavaPacket,
    ser::NetworkReadExt,
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};
use pumpkin_world::world::BlockFlags;
use rand::SeedableRng;
use std::sync::Arc;

const POSITION: BlockPos = BlockPos::new(8, 64, 8);

async fn fixture() -> (DeathTestWorld, Arc<World>, TestPlayer) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 6.5));
    (fixture, world, player)
}

fn put(world: &Arc<World>, position: BlockPos, state: BlockStateId) {
    world.set_block_state(
        &position,
        state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
}

fn hold(player: &TestPlayer, hand: Hand, count: u8, item: &'static Item) {
    player
        .player
        .inventory()
        .set_stack_in_hand(hand, ItemStack::new(count, item));
}

fn click(fixture: &DeathTestWorld, player: &TestPlayer, position: BlockPos, hand: Hand, y: f32) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(i32::from(hand == Hand::Left)),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, y, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.server,
        )
        .unwrap();
}

fn neighbor_state(fixture: &DeathTestWorld, world: &World, position: &BlockPos) -> BlockStateId {
    fixture.server.block_registry.get_state_for_neighbor_update(
        world,
        world.get_block(position),
        world.get_block_state_id(position),
        position,
        BlockDirection::East,
        &position.offset(BlockDirection::East.to_offset()),
        Block::AIR.default_state.id,
    )
}

fn can_place(fixture: &DeathTestWorld, world: &World, block: &Block) -> bool {
    fixture.server.block_registry.can_place_at(
        Some(&fixture.server),
        Some(world),
        world,
        None,
        block,
        block.default_state,
        &POSITION,
        None,
        None,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn survival_candle_merge_consumes_each_added_candle() {
    let (fixture, world, player) = fixture().await;
    for hand in [Hand::Right, Hand::Left] {
        put(&world, POSITION, Block::AIR.default_state.id);
        hold(&player, hand, 4, &Item::CANDLE);
        click(&fixture, &player, POSITION.down(), hand, 1.0);
        assert_eq!(
            player.player.inventory().get_stack_in_hand(hand).item_count,
            3
        );
        let mut properties =
            CandleLikeProperties::from_state_id(world.get_block_state_id(&POSITION));
        properties.lit = true;
        put(&world, POSITION, properties.to_state_id(&Block::CANDLE));
        for count in 2..=4 {
            click(&fixture, &player, POSITION, hand, 0.5);
            let properties =
                CandleLikeProperties::from_state_id(world.get_block_state_id(&POSITION));
            assert_eq!(properties.candles, count);
            assert!(properties.lit);
            assert_eq!(
                player.player.inventory().get_stack_in_hand(hand).item_count,
                4 - count
            );
        }
        hold(&player, hand, 1, &Item::CANDLE);
        click(&fixture, &player, POSITION, hand, 0.5);
        assert_eq!(
            player.player.inventory().get_stack_in_hand(hand).item_count,
            1
        );
        assert!(world.get_block_state(&POSITION.up()).is_air());
    }
    player.player.set_gamemode(GameMode::Creative);
    put(&world, POSITION, Block::CANDLE.default_state.id);
    hold(&player, Hand::Right, 1, &Item::CANDLE);
    click(&fixture, &player, POSITION, Hand::Right, 0.5);
    assert_eq!(
        CandleLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).candles,
        2
    );
    assert_eq!(player.player.inventory().held_item().item_count, 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sugar_cane_can_be_planted_on_dirt_beside_water() {
    let (fixture, world, player) = fixture().await;
    let adjacent = POSITION.down().offset(BlockDirection::East.to_offset());
    let waterlogged = Block::OAK_FENCE
        .default_state
        .set_waterlogged(true)
        .unwrap();
    for support in [
        Block::WATER.default_state,
        waterlogged,
        Block::FROSTED_ICE.default_state,
    ] {
        put(&world, POSITION, Block::AIR.default_state.id);
        put(&world, POSITION.down(), Block::DIRT.default_state.id);
        put(&world, adjacent, support.id);
        hold(&player, Hand::Right, 1, &Item::SUGAR_CANE);
        click(&fixture, &player, POSITION.down(), Hand::Right, 1.0);
        assert_eq!(
            world.get_block(&POSITION),
            &Block::SUGAR_CANE,
            "{}",
            support.id.to_block().name
        );
        assert!(player.player.inventory().held_item().is_empty());
    }
    put(&world, adjacent, Block::AIR.default_state.id);
    hold(&player, Hand::Right, 1, &Item::SUGAR_CANE);
    click(&fixture, &player, POSITION, Hand::Right, 1.0);
    assert_eq!(world.get_block(&POSITION.up()), &Block::SUGAR_CANE);
    assert!(player.player.inventory().held_item().is_empty());
    put(&world, POSITION.up(), Block::AIR.default_state.id);
    put(&world, POSITION, Block::AIR.default_state.id);
    hold(&player, Hand::Right, 1, &Item::SUGAR_CANE);
    click(&fixture, &player, POSITION.down(), Hand::Right, 1.0);
    assert!(world.get_block_state(&POSITION).is_air());
    assert_eq!(player.player.inventory().held_item().item_count, 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sugar_cane_new_segment_starts_at_age_zero() {
    let (fixture, world, _) = fixture().await;
    put(&world, POSITION.down(), Block::SUGAR_CANE.default_state.id);
    put(
        &world,
        POSITION,
        CactusLikeProperties { age: 15 }.to_state_id(&Block::SUGAR_CANE),
    );
    super::sugar_cane::SugarCaneBlock.random_tick(RandomTickArgs {
        world: &world,
        block: &Block::SUGAR_CANE,
        position: &POSITION,
    });
    assert_eq!(world.get_block(&POSITION.up()), &Block::SUGAR_CANE);
    assert_eq!(
        CactusLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age,
        0
    );
    assert_eq!(
        CactusLikeProperties::from_state_id(world.get_block_state_id(&POSITION.up())).age,
        0
    );
    // A new top on a two-high stalk needs sixteen eligible ticks of its own.
    put(&world, POSITION.down(), Block::DIRT.default_state.id);
    put(
        &world,
        POSITION.down().offset(BlockDirection::East.to_offset()),
        Block::WATER.default_state.id,
    );
    let top = POSITION.up();
    for age in 1..=15 {
        super::sugar_cane::SugarCaneBlock.random_tick(RandomTickArgs {
            world: &world,
            block: &Block::SUGAR_CANE,
            position: &top,
        });
        assert_eq!(
            CactusLikeProperties::from_state_id(world.get_block_state_id(&top)).age,
            age
        );
        assert!(world.get_block_state(&top.up()).is_air());
    }
    super::sugar_cane::SugarCaneBlock.random_tick(RandomTickArgs {
        world: &world,
        block: &Block::SUGAR_CANE,
        position: &top,
    });
    let third = top.up();
    assert_eq!(world.get_block(&third), &Block::SUGAR_CANE);
    put(
        &world,
        third,
        CactusLikeProperties { age: 15 }.to_state_id(&Block::SUGAR_CANE),
    );
    super::sugar_cane::SugarCaneBlock.random_tick(RandomTickArgs {
        world: &world,
        block: &Block::SUGAR_CANE,
        position: &third,
    });
    assert!(world.get_block_state(&third.up()).is_air());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bonemealing_sea_pickle_consumes_bone_meal() {
    let (fixture, world, player) = fixture().await;
    put(&world, POSITION.down(), Block::FARMLAND.default_state.id);
    put(&world, POSITION, Block::WHEAT.default_state.id);
    hold(&player, Hand::Right, 2, &Item::BONE_MEAL);
    click(&fixture, &player, POSITION, Hand::Right, 0.5);
    assert!(WheatLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age > 0);
    assert_eq!(player.player.inventory().held_item().item_count, 1);
    put(
        &world,
        POSITION.down(),
        Block::BRAIN_CORAL_BLOCK.default_state.id,
    );
    put(&world, POSITION, Block::SEA_PICKLE.default_state.id);
    click(&fixture, &player, POSITION, Hand::Right, 0.5);
    let properties = SeaPickleLikeProperties::from_state_id(world.get_block_state_id(&POSITION));
    assert_eq!(properties.pickles, 4);
    assert!(properties.waterlogged);
    assert!(player.player.inventory().held_item().is_empty());
    hold(&player, Hand::Right, 1, &Item::BONE_MEAL);
    for (waterlogged, support) in [
        (false, &Block::BRAIN_CORAL_BLOCK),
        (true, &Block::DEAD_BRAIN_CORAL_BLOCK),
    ] {
        put(&world, POSITION.down(), support.default_state.id);
        let properties = SeaPickleLikeProperties {
            pickles: 1,
            waterlogged,
        };
        put(&world, POSITION, properties.to_state_id(&Block::SEA_PICKLE));
        click(&fixture, &player, POSITION, Hand::Right, 0.5);
        assert_eq!(player.player.inventory().held_item().item_count, 1);
        assert_eq!(
            world.get_block_state_id(&POSITION),
            properties.to_state_id(&Block::SEA_PICKLE)
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sea_pickle_survives_normal_coral_support_after_neighbor_update() {
    let (fixture, world, _) = fixture().await;
    for support in [
        &Block::DIRT,
        &Block::BRAIN_CORAL_BLOCK,
        &Block::STONE,
        &Block::OAK_STAIRS,
    ] {
        put(&world, POSITION.down(), support.default_state.id);
        put(&world, POSITION, Block::SEA_PICKLE.default_state.id);
        assert!(
            can_place(&fixture, &world, &Block::SEA_PICKLE),
            "{}",
            support.name
        );
        assert_eq!(
            neighbor_state(&fixture, &world, &POSITION),
            Block::SEA_PICKLE.default_state.id
        );
        let adjacent = POSITION.offset(BlockDirection::East.to_offset());
        world.set_block_state(
            &adjacent,
            Block::STONE.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        assert_eq!(world.get_block(&POSITION), &Block::SEA_PICKLE);
        put(&world, adjacent, Block::AIR.default_state.id);
    }
    assert!(world.is_fluid_tick_scheduled(&POSITION, &Fluid::WATER));
    for support in [&Block::AIR, &Block::DIRT_PATH] {
        put(&world, POSITION.down(), support.default_state.id);
        put(&world, POSITION, Block::SEA_PICKLE.default_state.id);
        assert!(!can_place(&fixture, &world, &Block::SEA_PICKLE));
        assert_eq!(
            neighbor_state(&fixture, &world, &POSITION),
            Block::AIR.default_state.id
        );
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_hand_hit_extinguishes_candle_on_cake_without_eating_it() {
    let (fixture, world, player) = fixture().await;
    let lit = RedstoneOreLikeProperties { lit: true }.to_state_id(&Block::CANDLE_CAKE);
    for hand in [Hand::Right, Hand::Left] {
        put(&world, POSITION, lit);
        hold(&player, hand, 0, &Item::AIR);
        click(&fixture, &player, POSITION, hand, 0.5);
        assert_eq!(world.get_block_state_id(&POSITION), lit);
        click(&fixture, &player, POSITION, hand, 0.75);
        assert_eq!(world.get_block(&POSITION), &Block::CANDLE_CAKE);
        assert!(!RedstoneOreLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).lit);
    }
    put(&world, POSITION, lit);
    hold(&player, Hand::Right, 1, &Item::STICK);
    click(&fixture, &player, POSITION, Hand::Right, 0.75);
    assert_eq!(world.get_block_state_id(&POSITION), lit);
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    fixture.server.shutdown().await;
}

async fn assert_extinguish_sends_no_particles(block: &Block, lit_state: BlockStateId) {
    let (fixture, world, player) = fixture().await;
    let mut observer = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        player.player.clone(),
        observer.player.clone(),
    ]));
    put(&world, POSITION, lit_state);
    observer.take_packets();
    click(&fixture, &player, POSITION, Hand::Right, 0.75);
    assert_eq!(world.get_block_state_id(&POSITION), block.default_state.id);
    let packet_ids: Vec<_> = observer
        .take_packets()
        .iter()
        .map(|packet| packet.as_ref().get_var_int().unwrap().0)
        .collect();
    let version = pumpkin_data::packet::CURRENT_MC_VERSION;
    assert!(packet_ids.contains(&CSoundEffect::to_id(version)));
    // AbstractCandleBlock.extinguish calls Level.addParticle, a server-side no-op.
    assert!(!packet_ids.contains(&CParticle::to_id(version)));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extinguishing_candle_sends_no_particles_to_another_player() {
    let mut properties = CandleLikeProperties::default(&Block::CANDLE);
    properties.lit = true;
    assert_extinguish_sends_no_particles(&Block::CANDLE, properties.to_state_id(&Block::CANDLE))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extinguishing_candle_cake_sends_no_particles_to_another_player() {
    let lit_state = RedstoneOreLikeProperties { lit: true }.to_state_id(&Block::CANDLE_CAKE);
    assert_extinguish_sends_no_particles(&Block::CANDLE_CAKE, lit_state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wheat_planting_refuses_dark_farmland() {
    let (fixture, world, player) = fixture().await;
    put(&world, POSITION.down(), Block::FARMLAND.default_state.id);
    for (block, item) in [
        (&Block::WHEAT, &Item::WHEAT_SEEDS),
        (&Block::CARROTS, &Item::CARROT),
        (&Block::POTATOES, &Item::POTATO),
        (&Block::BEETROOTS, &Item::BEETROOT_SEEDS),
        (&Block::TORCHFLOWER_CROP, &Item::TORCHFLOWER_SEEDS),
    ] {
        for (brightness, accepted) in [(0, false), (7, false), (8, true), (15, true)] {
            put(&world, POSITION, Block::AIR.default_state.id);
            world.set_sky_light_level(&POSITION, brightness);
            assert_eq!(world.get_raw_brightness(&POSITION, 0), brightness);
            hold(&player, Hand::Right, 1, item);
            click(&fixture, &player, POSITION.down(), Hand::Right, 1.0);
            assert_eq!(
                world.get_block(&POSITION),
                if accepted { block } else { &Block::AIR }
            );
            assert_eq!(player.player.inventory().held_item().is_empty(), accepted);
        }
        world.set_sky_light_level(&POSITION, 7);
        assert_eq!(
            neighbor_state(&fixture, &world, &POSITION),
            Block::AIR.default_state.id
        );
    }
    put(&world, POSITION.down(), Block::SOUL_SAND.default_state.id);
    put(&world, POSITION, Block::AIR.default_state.id);
    world.set_sky_light_level(&POSITION, 0);
    hold(&player, Hand::Right, 1, &Item::NETHER_WART);
    click(&fixture, &player, POSITION.down(), Hand::Right, 1.0);
    assert_eq!(world.get_block(&POSITION), &Block::NETHER_WART);
    assert_eq!(
        neighbor_state(&fixture, &world, &POSITION),
        Block::NETHER_WART.default_state.id
    );
    assert!(player.player.inventory().held_item().is_empty());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crops_do_not_grow_at_brightness_eight() {
    use super::crop::{CropBlockBase, wheat::WheatBlock};
    let (fixture, world, _) = fixture().await;
    put(&world, POSITION.down(), Block::FARMLAND.default_state.id);
    for brightness in [8, 9] {
        put(&world, POSITION, Block::WHEAT.default_state.id);
        world.set_sky_light_level(&POSITION, brightness);
        assert_eq!(world.get_raw_brightness(&POSITION, 0), brightness);
        let mut random = rand::rngs::StdRng::seed_from_u64(0);
        for _ in 0..128 {
            WheatBlock.random_tick_with_rng(&world, &POSITION, &mut random);
        }
        let age = WheatLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age;
        if brightness == 8 {
            assert_eq!(age, 0);
        } else {
            assert!(age > 0);
        }
    }
    // Bonemeal admission is independent of the natural-growth light gate.
    put(&world, POSITION, Block::WHEAT.default_state.id);
    world.set_sky_light_level(&POSITION, 0);
    assert!(fixture.server.block_registry.bone_meal(
        &Block::WHEAT,
        &world,
        &POSITION,
        Block::WHEAT.default_state.id
    ));
    assert!(WheatLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age > 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stems_do_not_grow_at_brightness_eight() {
    let (fixture, world, _) = fixture().await;
    put(&world, POSITION.down(), Block::FARMLAND.default_state.id);
    for block in [&Block::MELON_STEM, &Block::PUMPKIN_STEM] {
        for brightness in [8, 9] {
            put(&world, POSITION, block.default_state.id);
            world.set_sky_light_level(&POSITION, brightness);
            let mut random = rand::rngs::StdRng::seed_from_u64(0);
            // Stop at the first age increment, before the fruit-placement roll.
            for _ in 0..128 {
                super::crop::gourds::stem::StemBlock::random_tick_with_rng(
                    &RandomTickArgs {
                        world: &world,
                        block,
                        position: &POSITION,
                    },
                    &mut random,
                );
                if WheatLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age > 0 {
                    break;
                }
            }
            let age = WheatLikeProperties::from_state_id(world.get_block_state_id(&POSITION)).age;
            assert_eq!(age, u8::from(brightness == 9));
        }
    }
    fixture.server.shutdown().await;
}
