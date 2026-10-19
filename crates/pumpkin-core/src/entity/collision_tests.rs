#![expect(
    clippy::unwrap_used,
    reason = "real movement fixtures must construct successfully"
)]

use super::{
    EntityBase,
    ai::goal::{Controls, Goal},
    attributes::{Modifier, ModifierOperation},
    mob::Mob,
};
use crate::{server::combat_test_support, world::World};
use pumpkin_data::{
    Block,
    attributes::Attributes,
    block_properties::{
        HorizontalFacing, OakSlabProperties, OakStairsProperties, SlabType, StairsShape,
    },
    entity::EntityType,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{
    sync::{Arc, atomic::Ordering::Relaxed},
    time::Duration,
};

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<crate::server::Server>,
    world: Arc<World>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        server.worlds.store(Arc::new(vec![world.clone()]));
        Self {
            _directory: directory,
            server,
            world,
        }
    }

    fn block(&self, x: usize, y: i32, z: usize, state: pumpkin_data::BlockStateId) {
        // Direct fixture placement avoids unrelated block survival/neighbor updates.
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&Vector2::new(0, 0))
            .unwrap();
        chunk.set_block_absolute_y(x, y, z, state);
    }

    fn lane(&self, from: &Block, to: &Block) {
        for x in 0..16 {
            for z in 0..16 {
                let block = if x < 6 { from } else { to };
                self.block(x, 63, z, block.default_state.id);
            }
        }
    }

    fn mob(&self, kind: &'static EntityType, pos: Vector3<f64>) -> Arc<dyn EntityBase> {
        let mob = super::r#type::from_type(kind, pos, &self.world, uuid::Uuid::new_v4());
        mob.get_entity().on_ground.store(true, Relaxed);
        self.world.add_entity_silent(mob.clone());
        mob
    }

    fn cow(&self, y: f64) -> Arc<dyn EntityBase> {
        self.mob(&EntityType::COW, Vector3::new(5.5, y, 8.5))
    }
}

// Synchronous world ticks cannot be interrupted by an async timeout.
fn bounded(action: impl FnOnce() + Send + 'static) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let _runtime = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => {}
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(error) => panic!("bounded collision regression did not finish: {error}"),
    }
}

fn move_across(mob: &dyn EntityBase) {
    mob.get_entity()
        .move_entity(mob, Vector3::new(1.0, -0.08, 0.0));
}

fn assert_supported(mob: &dyn EntityBase, y: f64) {
    let entity = mob.get_entity();
    assert!(
        (entity.pos.load().y - y).abs() < 1e-7,
        "feet: {:?}",
        entity.pos.load()
    );
    assert!(entity.on_ground.load(Relaxed));
    let support = entity.get_supporting_block_pos().unwrap();
    assert_eq!(support, BlockPos::new(6, (y - 0.001).floor() as i32, 8));
    assert!(
        entity.velocity.load().y.abs() < 1e-7,
        "step must not launch: {:?}",
        entity.velocity.load()
    );
}

// A controlled goal uses the installed navigator and normal controller/physics ticks.
struct WalkTo;
impl Goal for WalkTo {
    fn can_start(&mut self, _mob: &dyn Mob) -> bool {
        true
    }
    fn start(&mut self, mob: &dyn Mob) {
        let inner = mob.get_mob_entity();
        assert!(inner.navigator.lock().unwrap().move_to_coords(
            10.5,
            64.0,
            8.5,
            1.0,
            &inner.living_entity,
        ));
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

#[tokio::test]
async fn b1019_g25_spawned_goal_navigation_world_ticks_cross_path_lip_without_jump() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::DIRT_PATH, &Block::STONE);
        fixture
            .world
            .forced_chunks
            .lock()
            .unwrap()
            .insert(Vector2::new(0, 0));
        let cow = fixture.mob(&EntityType::COW, Vector3::new(4.5, 63.9375, 8.5));
        let mob = cow.get_mob().unwrap().get_mob_entity();
        mob.goals_selector.lock().unwrap().clear();
        mob.add_goal(0, WalkTo);
        cow.get_entity()
            .velocity
            .store(Vector3::new(0.0, -0.08, 0.0));
        let mut saw_path = false;
        let mut saw_rise = false;
        for _ in 0..100 {
            let before = cow.get_entity().pos.load();
            fixture.world.tick(&fixture.server);
            let after = cow.get_entity().pos.load();
            saw_path |= mob
                .navigator
                .lock()
                .unwrap()
                .get_path()
                .is_some_and(|p| p.get_node_count() > 0);
            assert!(
                !mob.living_entity.jumping.load(Relaxed),
                "path lip requested a jump"
            );
            assert_eq!(mob.living_entity.jumping_cooldown.load(Relaxed), 0);
            assert!(after.y <= 64.0 + 1e-7, "step became a jump: {after:?}");
            if after.y > before.y + 1e-7 {
                saw_rise = true;
                assert!(cow.get_entity().on_ground.load(Relaxed));
                assert!(cow.get_entity().vertical_collision.load(Relaxed));
                assert!(cow.get_entity().velocity.load().y <= 0.0);
            }
            if after.x > 8.0 {
                break;
            }
        }
        assert!(saw_path, "goal must produce a real nonempty path");
        assert!(
            cow.get_entity().pos.load().x > 8.0,
            "navigation remained trapped at the path lip: {:?}",
            cow.get_entity().pos.load()
        );
        assert!(saw_rise);
        assert!(cow.get_entity().on_ground.load(Relaxed));
        assert_eq!(cow.get_entity().get_supporting_block_pos().unwrap().0.y, 63);
        assert!(cow.get_entity().get_supporting_block_pos().unwrap().0.x >= 6);
    });
}

#[tokio::test]
async fn b1019_g25_real_move_steps_path_and_farmland_with_final_support() {
    bounded(|| {
        for from in [&Block::DIRT_PATH, &Block::FARMLAND] {
            let fixture = Fixture::new();
            fixture.lane(from, &Block::STONE);
            let cow = fixture.cow(63.9375);
            move_across(cow.as_ref());
            assert!(
                (cow.get_entity().pos.load().x - 6.5).abs() < 1e-7,
                "mob cannot cross {from:?}"
            );
            assert_supported(cow.as_ref(), 64.0);
            assert!(!cow.get_entity().horizontal_collision.load(Relaxed));
            assert!(cow.get_entity().vertical_collision.load(Relaxed));
        }
    });
}

#[tokio::test]
async fn b1019_g25_flat_and_downward_path_controls() {
    bounded(|| {
        for (from, to, y) in [
            (&Block::DIRT_PATH, &Block::DIRT_PATH, 63.9375),
            (&Block::STONE, &Block::DIRT_PATH, 64.0),
        ] {
            let fixture = Fixture::new();
            fixture.lane(from, to);
            let cow = fixture.cow(y);
            move_across(cow.as_ref());
            assert!((cow.get_entity().pos.load().x - 6.5).abs() < 1e-7);
            // A downward transition settles on the following normal movement, as in Entity.move.
            cow.get_entity()
                .move_entity(cow.as_ref(), Vector3::new(0.0, -0.08, 0.0));
            assert_supported(cow.as_ref(), 63.9375);
        }
    });
}

#[tokio::test]
async fn b1019_g25_slab_and_stair_collision_shape_candidates() {
    bounded(|| {
        let mut stairs = OakStairsProperties::default(&Block::OAK_STAIRS);
        stairs.facing = HorizontalFacing::East;
        stairs.shape = StairsShape::Straight;
        for state in [
            Block::OAK_SLAB.default_state.id,
            stairs.to_state_id(&Block::OAK_STAIRS),
        ] {
            let fixture = Fixture::new();
            fixture.lane(&Block::STONE, &Block::STONE);
            fixture.block(6, 64, 8, state);
            let cow = fixture.cow(64.0);
            cow.get_entity()
                .move_entity(cow.as_ref(), Vector3::new(0.4, -0.08, 0.0));
            assert!(
                (cow.get_entity().pos.load().x - 5.9).abs() < 1e-7,
                "shape blocked: {:?}",
                cow.get_entity().pos.load()
            );
            assert_supported(cow.as_ref(), 64.5);
        }
    });
}

#[tokio::test]
async fn b1019_g25_low_ceiling_and_too_tall_obstacles_stay_blocked() {
    bounded(|| {
        for ceiling in [false, true] {
            let fixture = Fixture::new();
            fixture.lane(&Block::STONE, &Block::STONE);
            let obstacle = if ceiling {
                &Block::OAK_SLAB
            } else {
                &Block::STONE
            };
            fixture.block(6, 64, 8, obstacle.default_state.id);
            if ceiling {
                for x in 4..8 {
                    let mut roof = OakSlabProperties::default(&Block::OAK_SLAB);
                    roof.r#type = SlabType::Top;
                    fixture.block(x, 65, 8, roof.to_state_id(&Block::OAK_SLAB));
                }
            }
            let cow = fixture.cow(64.0);
            move_across(cow.as_ref());
            assert!(cow.get_entity().pos.load().x < 6.0);
            assert!((cow.get_entity().pos.load().y - 64.0).abs() < 1e-7);
            assert!(cow.get_entity().horizontal_collision.load(Relaxed));
        }
    });
}

#[tokio::test]
async fn b1019_g25_zero_and_below_lip_step_height_stay_blocked() {
    bounded(|| {
        for height in [0.0, 0.03] {
            let fixture = Fixture::new();
            fixture.lane(&Block::DIRT_PATH, &Block::STONE);
            let cow = fixture.cow(63.9375);
            cow.get_living_entity()
                .unwrap()
                .set_attribute_base(&Attributes::STEP_HEIGHT, height);
            move_across(cow.as_ref());
            assert!(cow.get_entity().pos.load().x < 6.0);
            assert!((cow.get_entity().pos.load().y - 63.9375).abs() < 1e-7);
        }
    });
}

#[tokio::test]
async fn b1019_g25_effective_modifier_and_enderman_one_block_step() {
    bounded(|| {
        for enderman in [false, true] {
            let fixture = Fixture::new();
            fixture.lane(
                if enderman {
                    &Block::STONE
                } else {
                    &Block::DIRT_PATH
                },
                &Block::STONE,
            );
            let mob = if enderman {
                fixture.block(6, 64, 8, Block::STONE.default_state.id);
                fixture.mob(&EntityType::ENDERMAN, Vector3::new(5.5, 64.0, 8.5))
            } else {
                let cow = fixture.cow(63.9375);
                let living = cow.get_living_entity().unwrap();
                living.set_attribute_base(&Attributes::STEP_HEIGHT, 0.0);
                living
                    .attributes
                    .write()
                    .unwrap()
                    .get_mut(&Attributes::STEP_HEIGHT.id)
                    .unwrap()
                    .add_or_replace_modifier(Modifier {
                        id: "b1019_g25:step".to_owned(),
                        amount: 0.0625,
                        operation: ModifierOperation::Add,
                        permanent: false,
                    });
                cow
            };
            move_across(mob.as_ref());
            assert!((mob.get_entity().pos.load().x - 6.5).abs() < 1e-7);
            assert_supported(mob.as_ref(), if enderman { 65.0 } else { 64.0 });
        }
    });
}

#[tokio::test]
async fn b1019_g25_airborne_horizontal_collision_does_not_step() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::DIRT_PATH, &Block::STONE);
        let cow = fixture.cow(63.96);
        cow.get_entity().on_ground.store(false, Relaxed);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(1.0, 0.0, 0.0));
        assert!(cow.get_entity().pos.load().x < 6.0);
        assert!((cow.get_entity().pos.load().y - 63.96).abs() < 1e-7);
        assert!(!cow.get_entity().on_ground.load(Relaxed));
        assert!(cow.get_entity().get_supporting_block_pos().is_none());
    });
}

#[tokio::test]
async fn b1019_g25_landing_retry_accounts_for_distance_to_ground() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::DIRT_PATH, &Block::STONE);
        let cow = fixture.cow(64.1375);
        cow.get_entity().on_ground.store(false, Relaxed);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(1.0, -0.3, 0.0));
        assert!((cow.get_entity().pos.load().x - 6.5).abs() < 1e-7);
        assert_supported(cow.as_ref(), 64.0);
    });
}

#[tokio::test]
async fn b1019_g25_landing_step_keeps_incident_velocity_for_slime_restitution() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::DIRT_PATH, &Block::SLIME_BLOCK);
        let cow = fixture.cow(64.1375);
        cow.get_entity().on_ground.store(false, Relaxed);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(1.0, -0.3, 0.0));
        assert!((cow.get_entity().pos.load().x - 6.5).abs() < 1e-7);
        assert!((cow.get_entity().pos.load().y - 64.0).abs() < 1e-7);
        assert!((cow.get_entity().velocity.load().y - 0.3).abs() < 1e-7);
    });
}

#[tokio::test]
async fn b1019_g25_diagonal_wall_slide_and_shape_edge_tolerance() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::STONE, &Block::STONE);
        for z in 0..16 {
            fixture.block(6, 64, z, Block::STONE.default_state.id);
        }
        let cow = fixture.cow(64.0);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(1.0, -0.08, 0.5));
        assert!(cow.get_entity().bounding_box.load().max.x <= 6.0 + 1e-7);
        assert!((cow.get_entity().pos.load().z - 9.0).abs() < 1e-7);
        assert!(cow.get_entity().horizontal_collision.load(Relaxed));

        let edge = Fixture::new();
        edge.lane(&Block::STONE, &Block::STONE);
        edge.block(6, 64, 8, Block::OAK_SLAB.default_state.id);
        let cow = edge.cow(64.5 - 5.0e-8);
        cow.get_living_entity()
            .unwrap()
            .set_attribute_base(&Attributes::STEP_HEIGHT, 0.0);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(1.0, 0.0, 0.0));
        assert!((cow.get_entity().pos.load().x - 6.5).abs() < 1e-7);
        assert!(!cow.get_entity().horizontal_collision.load(Relaxed));
    });
}

#[tokio::test]
async fn b1019_g25_off_edge_support_lasts_until_the_next_falling_move() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::STONE, &Block::AIR);
        let cow = fixture.cow(64.0);
        move_across(cow.as_ref());
        assert!((cow.get_entity().pos.load().x - 6.5).abs() < 1e-7);
        assert!(cow.get_entity().on_ground.load(Relaxed));
        assert_eq!(
            cow.get_entity().get_supporting_block_pos(),
            Some(BlockPos::new(5, 63, 8))
        );
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(0.0, -0.08, 0.0));
        assert!(!cow.get_entity().on_ground.load(Relaxed));
        assert!(cow.get_entity().get_supporting_block_pos().is_none());
        assert!((cow.get_entity().pos.load().y - 63.92).abs() < 1e-7);
    });
}

#[tokio::test]
async fn b1019_g25_finite_and_expanded_sweep_caps() {
    bounded(|| {
        let fixture = Fixture::new();
        fixture.lane(&Block::DIRT_PATH, &Block::STONE);
        let cow = fixture.cow(63.9375);
        let start = cow.get_entity().pos.load();
        for motion in [
            Vector3::new(f64::NAN, 0.0, 0.0),
            Vector3::new(129.0, 0.0, 0.0),
            Vector3::new(100.0, 100.0, 100.0),
        ] {
            cow.get_entity().velocity.store(motion);
            cow.get_entity().move_entity(cow.as_ref(), motion);
            assert_eq!(cow.get_entity().pos.load(), start);
            assert_eq!(cow.get_entity().velocity.load(), Vector3::default());
        }
        for height in [129.0, f64::MAX, f64::NAN] {
            cow.get_living_entity()
                .unwrap()
                .set_attribute_base(&Attributes::STEP_HEIGHT, height);
            move_across(cow.as_ref());
            assert!(cow.get_entity().pos.load().x < 6.0);
            assert!((cow.get_entity().pos.load().y - start.y).abs() < 1e-7);
        }
        // Each component is admitted, but the upward-expanded retry exceeds the cell budget.
        cow.get_living_entity()
            .unwrap()
            .set_attribute_base(&Attributes::STEP_HEIGHT, 128.0);
        cow.get_entity()
            .move_entity(cow.as_ref(), Vector3::new(128.0, -0.08, 128.0));
        assert!((cow.get_entity().pos.load().y - start.y).abs() < 1e-7);
    });
}
