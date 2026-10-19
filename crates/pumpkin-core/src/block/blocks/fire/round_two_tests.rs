use super::*;
use crate::plugin::api::events::block::block_spread::BlockSpreadEvent;
use pumpkin_data::entity::EntityType;

struct ReplaceBurnTarget {
    position: BlockPos,
    replacement: BlockStateId,
    calls: AtomicUsize,
}

impl EventHandler<BlockBurnEvent> for ReplaceBurnTarget {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockBurnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block_pos == self.position {
            self.calls.fetch_add(1, Relaxed);
            event.world.set_block_state(
                &self.position,
                self.replacement,
                BlockFlags::FORCE_STATE | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
            );
        }
        Box::pin(async {})
    }
}

struct ReplaceSpreadTarget {
    position: BlockPos,
    replacement: BlockStateId,
    calls: AtomicUsize,
}

impl EventHandler<BlockSpreadEvent> for ReplaceSpreadTarget {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockSpreadEvent,
    ) -> BoxFuture<'a, ()> {
        if event.target_pos == self.position {
            self.calls.fetch_add(1, Relaxed);
            event.world.set_block_state(
                &self.position,
                self.replacement,
                BlockFlags::FORCE_STATE | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
            );
        }
        Box::pin(async {})
    }
}

fn seed_for_air_spread(rate: i32) -> u64 {
    (0..2_000_000)
        .find(|seed| {
            let mut rng = StdRng::seed_from_u64(*seed);
            let _delay: i32 = rng.random_range(0..10);
            let _age = rng.random_range(0..3);
            for chance in [300, 300, 250, 250, 300, 300] {
                let _burn = rng.random_range(0..chance);
            }
            rng.random_range(0..rate) == 0
        })
        .expect("bounded seeded search finds an air-spread roll")
}

fn seed_for_zero_radius_wood_loss(burn_odds: i32) -> u64 {
    (0..2_000_000)
        .find(|seed| {
            let mut spread_rng = StdRng::seed_from_u64(*seed);
            let _delay: i32 = spread_rng.random_range(0..10);
            let _age = spread_rng.random_range(0..3);
            for chance in [300, 300, 250, 250, 300, 300] {
                let _burn = spread_rng.random_range(0..chance);
            }
            if spread_rng.random_range(0..100) != 0 {
                return false;
            }
            let _spread_age = spread_rng.random_range(0..5);

            let mut target_rng = StdRng::seed_from_u64(*seed);
            let _delay: i32 = target_rng.random_range(0..10);
            let _age = target_rng.random_range(0..3);
            for (index, chance) in [300, 300, 250, 250, 300, 300].into_iter().enumerate() {
                let burn = target_rng.random_range(0..chance);
                if index == 4 && burn >= burn_odds {
                    return false;
                }
            }
            true
        })
        .expect("bounded seeded search finds air spread and a later wood burn")
}

fn seed_for_tnt_burn(burn_odds: i32) -> u64 {
    (0..2_000_000)
        .find(|seed| {
            let mut rng = StdRng::seed_from_u64(*seed);
            let _delay: i32 = rng.random_range(0..10);
            let _age = rng.random_range(0..3);
            rng.random_range(0..300) < burn_odds
        })
        .expect("bounded seeded search finds a TNT burn roll")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn radius_zero_b1019_g10_with_centered_players_preserves_two_cell_wood_target() {
    let fixture = FireFixture::new();
    let source = BlockPos::new(8, 64, 8);
    let target = source.north();
    let wood = target.north();
    fixture.set_fire_radius(0);
    fixture.set_block(source.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.down(), Block::NETHERRACK.default_state.id);
    let wood_state = oak_slab(false);
    fixture.set_block(wood, wood_state);
    let source_player = TestPlayer::new(&fixture.world);
    source_player
        .player
        .get_entity()
        .set_pos(source.to_centered_f64());
    let target_player = TestPlayer::new(&fixture.world);
    target_player
        .player
        .get_entity()
        .set_pos(target.to_centered_f64());
    fixture.world.players.store(Arc::new(vec![
        source_player.player.clone(),
        target_player.player.clone(),
    ]));
    fixture.set_fire(source, 0);

    let spread_seed = seed_for_zero_radius_wood_loss(i32::from(
        Block::OAK_SLAB.flammable.as_ref().unwrap().burn_chance,
    ));
    fixture.schedule_fire(source);
    fixture.tick_with_seed(spread_seed);

    fixture.set_block(source, Block::AIR.default_state.id);
    let next_tick = fixture.world.get_world_age() + 40;
    fixture.world.level_time.lock().unwrap().world_age = next_tick;
    fixture.world.level.set_game_time(next_tick);
    fixture.tick_with_seed(spread_seed);

    assert_eq!(fixture.world.get_block_state_id(&wood), wood_state);
    assert_eq!(
        fixture.world.get_block_state_id(&target),
        Block::AIR.default_state.id
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_fire_b1019_g10_gates_from_source_position() {
    let fixture = FireFixture::new();
    let source = BlockPos::new(8, 64, 8);
    let target = source.up().up().up().up();
    fixture.set_fire_radius(4);
    fixture.set_block(source.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.east(), oak_slab(false));
    let player = TestPlayer::new(&fixture.world);
    player.player.get_entity().set_pos(target.to_centered_f64());
    fixture.set_fire(source, 0);

    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_air_spread(400));

    assert_eq!(
        fixture.world.get_block_state_id(&target),
        Block::AIR.default_state.id
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_fire_b1019_g10_spreads_to_air_when_unrestricted() {
    let fixture = FireFixture::new();
    let source = BlockPos::new(8, 64, 8);
    let target = source.north();
    fixture.set_fire_radius(-1);
    fixture.set_block(source.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.north(), oak_slab(false));
    fixture.set_fire(source, 0);

    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_air_spread(100));

    assert_eq!(fixture.world.get_block(&target), &Block::FIRE);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_burn_b1019_g10_preserves_callback_replacement() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let source = BlockPos::new(8, 64, 8);
    let target = source.east();
    fixture.set_block(source.down(), Block::STONE.default_state.id);
    fixture.set_block(target, oak_slab(false));
    fixture.set_fire(source, 0);
    let replacement = oak_slab(true);
    let handler = Arc::new(ReplaceBurnTarget {
        position: target,
        replacement,
        calls: AtomicUsize::new(0),
    });
    fixture.server.plugin_manager.register::<BlockBurnEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );

    let burn_odds = i32::from(Block::OAK_SLAB.flammable.as_ref().unwrap().burn_chance);
    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_cancelled_burn(burn_odds));

    assert_eq!(handler.calls.load(Relaxed), 1);
    assert_eq!(fixture.world.get_block_state_id(&target), replacement);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn block_spread_b1019_g10_preserves_callback_replacement() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let source = BlockPos::new(8, 64, 8);
    let target = source.north();
    fixture.set_block(source.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.down(), Block::NETHERRACK.default_state.id);
    fixture.set_block(target.north(), oak_slab(false));
    fixture.set_fire(source, 0);
    let replacement = Block::STONE.default_state.id;
    let handler = Arc::new(ReplaceSpreadTarget {
        position: target,
        replacement,
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<BlockSpreadEvent, _>(handler.clone(), EventPriority::Normal, true);

    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_air_spread(100));

    assert_eq!(handler.calls.load(Relaxed), 1);
    assert_eq!(fixture.world.get_block_state_id(&target), replacement);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adjacent_fire_b1019_g10_primes_tnt_once() {
    let fixture = FireFixture::new();
    fixture.set_fire_radius(-1);
    let source = BlockPos::new(8, 64, 8);
    let tnt = source.east();
    fixture.set_block(source.down(), Block::STONE.default_state.id);
    fixture.set_block(tnt, Block::TNT.default_state.id);
    fixture.set_fire(source, 0);

    let burn_odds = i32::from(Block::TNT.flammable.as_ref().unwrap().burn_chance);
    fixture.schedule_fire(source);
    fixture.tick_with_seed(seed_for_tnt_burn(burn_odds));

    assert_ne!(fixture.world.get_block(&tnt), &Block::TNT);
    assert_eq!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::TNT)
            .count(),
        1,
        "one admitted TNT burnout must create exactly one primed TNT"
    );
    fixture.shutdown().await;
}
