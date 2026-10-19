use std::time::{Duration, Instant};

use pumpkin_macros::pumpkin_block;

use crate::{
    block::{
        BlockBehaviour, GetStateForNeighborUpdateArgs, blocks::bed::BedBlock,
        registry::BlockRegistry,
    },
    plugin::api::events::{
        block::block_explode::BlockExplodeEvent, entity::entity_damage::EntityDamageEvent,
    },
};

use super::{
    Arc, AtomicUsize, Block, BlockDirection, BlockFlags, BlockStateId, Dimension,
    EntityExplodeEvent, FOOT, Fixture, HEAD, Item, Relaxed, SUseItemOn, VarInt, Vector3,
    explosion_packets,
};

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "bed interleaving did not reach its seam"
        );
        std::thread::yield_now();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_partial_unsafe_use_explodes_after_real_strict_setblock() {
    let mut fixture = Fixture::new(Dimension::THE_NETHER);
    fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
    fixture
        .client
        .player
        .permission_lvl
        .store(pumpkin_util::PermissionLvl::Four);
    let explosions = Arc::new(AtomicUsize::new(0));
    let seen = explosions.clone();
    fixture.register::<EntityExplodeEvent>(move |event| {
        assert_eq!(event.position, HEAD.to_centered_f64());
        assert_eq!(event.yield_rate, 5.0);
        seen.fetch_add(1, Relaxed);
    });
    let damage = Arc::new(AtomicUsize::new(0));
    let seen = damage.clone();
    let victim = fixture.client.player.entity_id();
    fixture.register::<EntityDamageEvent>(move |event| {
        if event.entity_id == victim {
            assert!(event.damage > 0.0);
            seen.fetch_add(1, Relaxed);
            // Admit deterministic nonfatal damage through the explosion's real damage entry.
            event.damage = 1.0;
        }
    });
    // Keep the competing writer observable after the explosion's entity-damage phase.
    fixture.register::<BlockExplodeEvent>(|event| event.cancelled = true);
    fixture.world.bed_half_write_test.store(1, Relaxed);
    let player = fixture.client.player.clone();
    let server = fixture.server.clone();
    let use_bed = tokio::task::spawn_blocking(move || {
        let client = player.client.java().unwrap();
        client
            .handle_use_item_on(
                &player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: FOOT,
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(2),
                },
                &server,
            )
            .unwrap();
    });
    until(|| fixture.world.bed_half_write_test.load(Relaxed) == 2);
    assert!(fixture.world.get_block(&HEAD).is_air());
    assert_eq!(fixture.world.get_block(&FOOT), &Block::WHITE_BED);
    let player = fixture.client.player.clone();
    let server = fixture.server.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let source = player.get_command_source(&server);
        server
            .command_dispatcher
            .load()
            .execute_input("setblock 5 64 5 stone strict", &source)
            .unwrap()
    });
    until(|| fixture.world.get_block(&FOOT) == &Block::STONE);
    fixture.world.bed_half_write_test.store(3, Relaxed);
    tokio::time::timeout(Duration::from_secs(20), use_bed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(20), writer)
            .await
            .unwrap()
            .unwrap(),
        1
    );
    assert_eq!(explosions.load(Relaxed), 1);
    assert_eq!(damage.load(Relaxed), 1);
    assert_eq!(fixture.client.player.living_entity.health.load(), 19.0);
    assert!(fixture.world.get_block(&HEAD).is_air());
    assert_eq!(fixture.world.get_block(&FOOT), &Block::STONE);
    assert_eq!(explosion_packets(&fixture.client.take_packets()), 1);
    fixture.awake();
    fixture.finish().await;
}

#[pumpkin_block("minecraft:stone")]
struct ShapeProbe {
    changed: Arc<AtomicUsize>,
}

impl BlockBehaviour for ShapeProbe {
    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        if [FOOT.up(), HEAD.up()].contains(args.position)
            && args.direction == BlockDirection::Down
            && args.neighbor_state_id == BlockStateId::AIR
        {
            self.changed.fetch_add(1, Relaxed);
            BlockStateId::AIR
        } else {
            args.state_id
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_unsafe_use_propagates_shape_changes_when_explosion_is_cancelled() {
    let changed = Arc::new(AtomicUsize::new(0));
    let mut registry = BlockRegistry::default();
    registry.register(BedBlock);
    registry.register(ShapeProbe {
        changed: changed.clone(),
    });
    let mut fixture = Fixture::with_registry(Dimension::THE_NETHER, Some(Arc::new(registry)));
    fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
    for position in [FOOT.up(), HEAD.up()] {
        fixture.world.set_block_state(
            &position,
            Block::STONE.default_state.id,
            BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }
    fixture.register::<EntityExplodeEvent>(|event| event.cancelled = true);
    fixture.use_on(FOOT);
    assert_eq!(changed.load(Relaxed), 2);
    for position in [FOOT, HEAD, FOOT.up(), HEAD.up()] {
        assert!(fixture.world.get_block(&position).is_air());
    }
    assert_eq!(explosion_packets(&fixture.client.take_packets()), 0);
    fixture.finish().await;
}
