#![expect(
    clippy::unwrap_used,
    reason = "Landing lifecycle regression fixtures must be valid"
)]

use super::movement_landing_tests::{MovementTest, on_damage};
use crate::{
    block::{
        BlockBehaviour, BlockMetadata, OnLandedUponArgs,
        blocks::{bed::BedBlock, dripstone::DripstoneBlock, hay::HayBlock},
        registry::BlockRegistry,
    },
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    world::World,
};
use pumpkin_data::{
    Block, BlockId,
    block_properties::{PointedDripstoneLikeProperties, SpeleothemThickness, VerticalDirection},
    damage::DamageType,
    packet::clientbound::play::SOUND,
    statistic::CustomStatistic,
};
use pumpkin_protocol::{
    java::server::play::{FLAG_ON_GROUND, SPlayerPosition},
    ser::NetworkReadExt,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{
    num::NonZeroU8,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

const BOUND: Duration = Duration::from_secs(10);
const SUPPORT: BlockPos = BlockPos::new(4, 0, 4);

struct OrdinaryBlock;
impl BlockBehaviour for OrdinaryBlock {}

#[derive(Clone, Copy)]
enum LandingPath {
    Default,
    Hay,
    Bed,
    Stalagmite,
}

impl LandingPath {
    fn block(self) -> &'static Block {
        match self {
            Self::Default => &Block::STONE,
            Self::Hay => &Block::HAY_BLOCK,
            Self::Bed => &Block::WHITE_BED,
            Self::Stalagmite => &Block::POINTED_DRIPSTONE,
        }
    }

    fn behavior(self) -> Arc<dyn BlockBehaviour> {
        match self {
            Self::Default => Arc::new(OrdinaryBlock),
            Self::Hay => Arc::new(HayBlock),
            Self::Bed => Arc::new(BedBlock),
            Self::Stalagmite => Arc::new(DripstoneBlock),
        }
    }

    fn damage(self) -> f32 {
        match self {
            Self::Default => 6.0,
            Self::Hay | Self::Bed => 1.0,
            Self::Stalagmite => 17.0,
        }
    }

    fn centimeters(self) -> i32 {
        match self {
            Self::Default | Self::Hay => 900,
            Self::Bed => 450,
            Self::Stalagmite => 1150,
        }
    }
}

struct GatedLanding {
    behavior: Arc<dyn BlockBehaviour>,
    reached: crossbeam::channel::Sender<()>,
    resume: crossbeam::channel::Receiver<()>,
    calls: Arc<AtomicUsize>,
}

impl BlockMetadata for GatedLanding {
    fn ids() -> Box<[BlockId]> {
        Box::new([
            Block::STONE.id,
            Block::HAY_BLOCK.id,
            Block::WHITE_BED.id,
            Block::POINTED_DRIPSTONE.id,
        ])
    }
}

impl BlockBehaviour for GatedLanding {
    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        assert_eq!(self.calls.fetch_add(1, Relaxed), 0);
        assert_eq!(args.fall_distance, 9.0, "final descending packet counted");
        assert_eq!(
            args.entity
                .get_living_entity()
                .unwrap()
                .fall_distance
                .load(),
            9.0
        );
        self.reached.send_timeout((), BOUND).unwrap();
        self.resume.recv_timeout(BOUND).unwrap();
        self.behavior.on_landed_upon(args);
    }
}

fn gated_fixture(path: LandingPath, gate: GatedLanding) -> (MovementTest, f64) {
    let mut fixture = MovementTest::new(path.block(), false);
    let mut registry = BlockRegistry::default();
    registry.register(gate);
    let world = Arc::new(World::load(
        fixture.world.level.clone(),
        fixture.world.level_info.clone(),
        fixture.world.dimension.clone(),
        Arc::new(registry),
        Arc::downgrade(&fixture.server),
    ));
    if matches!(path, LandingPath::Stalagmite) {
        let mut properties = PointedDripstoneLikeProperties::default(path.block());
        properties.vertical_direction = VerticalDirection::Up;
        properties.thickness = SpeleothemThickness::Tip;
        world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap()
            .set_block_absolute_y(4, 0, 4, properties.to_state_id(path.block()));
    }
    let landing_y = world
        .get_block_state(&SUPPORT)
        .get_block_collision_shapes()
        .map(|shape| shape.max.y)
        .fold(0.0f64, f64::max);
    assert!(landing_y > 0.0);
    fixture.client = TestPlayer::new(&world);
    fixture.player().config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZeroU8::new(2).unwrap();
        config
    });
    fixture
        .player()
        .get_entity()
        .set_pos(Vector3::new(4.5, landing_y + 9.0, 4.5));
    fixture
        .player()
        .get_entity()
        .on_ground
        .store(false, Relaxed);
    if matches!(path, LandingPath::Stalagmite) {
        // The actual dripstone override must retain STALAGMITE rather than FALL.
        fixture
            .player()
            .get_entity()
            .set_damage_immunity(DamageType::FALL, true);
    }
    fixture.server.worlds.store(Arc::new(vec![world.clone()]));
    fixture.world = world;
    (fixture, landing_y)
}

async fn gated_landing(path: LandingPath, replace_life: bool) {
    let (reached, entered) = crossbeam::channel::bounded(1);
    let (resume, released) = crossbeam::channel::bounded(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let (mut fixture, landing_y) = gated_fixture(
        path,
        GatedLanding {
            behavior: path.behavior(),
            reached,
            resume: released,
            calls: calls.clone(),
        },
    );
    let damage_events = Arc::new(AtomicUsize::new(0));
    let seen = damage_events.clone();
    on_damage(&fixture.server, move |event| {
        seen.fetch_add(1, Relaxed);
        assert_eq!(event.damage, path.damage());
        assert_eq!(
            event.damage_type,
            if matches!(path, LandingPath::Stalagmite) {
                DamageType::STALAGMITE
            } else {
                DamageType::FALL
            }
        );
    });
    fixture.position(landing_y + 0.75, false, false);
    assert_eq!(fixture.player().living_entity.fall_distance.load(), 8.25);
    let player = fixture.player().clone();
    let server = fixture.server.clone();
    let landing = tokio::task::spawn_blocking(move || {
        let client = player.client.java().unwrap();
        player.living_entity.with_damage_owned(|| {
            client.handle_position(
                &player,
                &server,
                &SPlayerPosition {
                    position: Vector3::new(4.5, landing_y, 4.5),
                    collision: FLAG_ON_GROUND,
                },
            );
        });
    });
    entered.recv_timeout(BOUND).unwrap();
    assert_eq!(calls.load(Relaxed), 1, "registered landing gate reached");
    assert_eq!(
        fixture.player().get_custom_stat(CustomStatistic::FallOneCm),
        0
    );
    if replace_life {
        let player = fixture.player().clone();
        let reset = tokio::task::spawn_blocking(move || {
            player.living_entity.reset_state();
            player.set_health(18.0);
            player.living_entity.hurt_cooldown.store(0, Relaxed);
            player.living_entity.fall_distance.store(7.0);
        });
        tokio::time::timeout(BOUND, reset).await.unwrap().unwrap();
    }
    fixture.client.take_packets();
    resume.send_timeout((), BOUND).unwrap();
    tokio::time::timeout(BOUND, landing).await.unwrap().unwrap();

    let sounds = fixture
        .client
        .take_packets()
        .iter()
        .filter(|packet| {
            let mut data = packet.as_ref();
            data.get_var_int().unwrap().0 == SOUND.0
        })
        .count();
    if replace_life {
        assert_eq!(fixture.player().living_entity.health.load(), 18.0);
        assert_eq!(fixture.player().living_entity.fall_distance.load(), 7.0);
        assert_eq!(
            fixture.player().get_custom_stat(CustomStatistic::FallOneCm),
            0
        );
        assert_eq!(damage_events.load(Relaxed), 0);
        assert_eq!(
            sounds, 0,
            "old landing emitted a replacement-life fall sound"
        );
    } else {
        fixture.assert_landed(20.0 - path.damage(), path.centimeters());
        assert_eq!(damage_events.load(Relaxed), 1);
        assert_eq!(sounds, 1);
        fixture.position(landing_y, true, false);
        fixture.assert_landed(20.0 - path.damage(), path.centimeters());
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(damage_events.load(Relaxed), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_landing_callback_cannot_damage_replacement_life() {
    gated_landing(LandingPath::Default, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_hay_callback_cannot_damage_replacement_life() {
    gated_landing(LandingPath::Hay, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_bed_callback_cannot_damage_replacement_life() {
    gated_landing(LandingPath::Bed, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_stalagmite_callback_cannot_damage_replacement_life() {
    gated_landing(LandingPath::Stalagmite, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_default_callback_same_life_counts_final_descent_once() {
    gated_landing(LandingPath::Default, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_hay_callback_same_life_keeps_reduction() {
    gated_landing(LandingPath::Hay, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_bed_callback_same_life_keeps_reduction() {
    gated_landing(LandingPath::Bed, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_stalagmite_callback_same_life_keeps_source() {
    gated_landing(LandingPath::Stalagmite, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g07_fall_reentrant_descent_and_api_controls() {
    let fixture = MovementTest::new(&Block::STONE, false);
    let player = fixture.player().clone();
    let server = fixture.server.clone();
    let callbacks = Arc::new(AtomicUsize::new(0));
    let seen = callbacks.clone();
    on_damage(&fixture.server, move |_| {
        if seen.fetch_add(1, Relaxed) != 0 {
            return;
        }
        let client = player.client.java().unwrap();
        for y in [2.0, 1.5] {
            let position = Vector3::new(4.5, y, 4.5);
            client.handle_position(
                &player,
                &server,
                &SPlayerPosition {
                    position,
                    collision: 0,
                },
            );
            assert_eq!(player.get_entity().pos.load(), position);
            assert!(!player.get_entity().on_ground.load(Relaxed));
        }
    });
    fixture.position(1.6, false, false);
    fixture.client.client().handle_position(
        fixture.player(),
        &fixture.server,
        &SPlayerPosition {
            position: Vector3::new(4.5, 1.0, 4.5),
            collision: FLAG_ON_GROUND,
        },
    );
    let player = fixture.player();
    assert_eq!(player.get_entity().pos.load(), Vector3::new(4.5, 1.5, 4.5));
    assert_eq!(player.living_entity.fall_distance.load(), 0.5);
    assert_eq!(player.living_entity.health.load(), 19.0);
    fixture.position(1.0, true, false);
    fixture.assert_landed(19.0, 400);
    assert_eq!(callbacks.load(Relaxed), 1);

    player.set_health(20.0);
    player.living_entity.hurt_cooldown.store(0, Relaxed);
    player
        .living_entity
        .handle_fall_damage(player.as_ref(), 6.0, 1.0);
    fixture.assert_landed(17.0, 1000);
    player.living_entity.hurt_cooldown.store(0, Relaxed);
    player
        .living_entity
        .handle_fall_damage_from(player.as_ref(), 9.0, 0.2, DamageType::FALL);
    fixture.assert_landed(16.0, 1900);
    assert_eq!(callbacks.load(Relaxed), 3);
}
