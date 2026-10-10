use crate::entity::{
    Entity, EntityBase,
    ai::goal::{Goal, melee_attack::MeleeAttackGoal},
    death_test_world::DeathTestWorld,
    mob::{Mob, bat::BatEntity, creaking::CreakingEntity, vex::VexEntity},
    passive::{goat::GoatEntity, turtle::TurtleEntity},
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::{Arc, atomic::Ordering::Relaxed};

fn floor(world: &crate::world::World) -> Arc<ChunkData> {
    let chunk = ChunkData::empty_sync(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_absolute_y(x, 59, z, Block::STONE.default_state.id);
        }
    }
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), chunk.clone());
    chunk
}

fn tick(fixture: &DeathTestWorld, entity: &dyn EntityBase) {
    fixture.world().level_time.lock().unwrap().world_age += 1;
    entity.tick(entity, &fixture.server);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn brain_species_swim_goal_requests_jumps_in_water() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = floor(&world);
    for y in 60..=64 {
        chunk.set_block_absolute_y(4, y, 4, Block::WATER.default_state.id);
    }
    for kind in [
        &EntityType::VILLAGER,
        &EntityType::WARDEN,
        &EntityType::CREAKING,
        &EntityType::ALLAY,
        &EntityType::GOAT,
        &EntityType::CAMEL,
        &EntityType::ARMADILLO,
        &EntityType::SNIFFER,
        &EntityType::BREEZE,
    ] {
        let entity = crate::entity::r#type::from_type(
            kind,
            Vector3::new(4.5, 60.0, 4.5),
            &world,
            uuid::Uuid::new_v4(),
        );
        let mob = entity.get_mob().unwrap().get_mob_entity();
        mob.persistence_required.store(true, Relaxed);
        let mut jumped = false;
        for _ in 0..40 {
            // Keep the mob submerged while checking the production goal selector and controls.
            entity.get_entity().set_pos(Vector3::new(4.5, 60.0, 4.5));
            tick(&fixture, entity.as_ref());
            jumped |= mob.living_entity.jumping.load(Relaxed);
        }
        assert!(jumped, "{} never requested a swim jump", kind.resource_name);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goat_stays_afloat_over_full_ticks() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = floor(&world);
    for x in 0..16 {
        for z in 0..16 {
            for y in 60..=63 {
                chunk.set_block_absolute_y(x, y, z, Block::WATER.default_state.id);
            }
        }
    }
    let goat = GoatEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 61.0, 4.5),
        &EntityType::GOAT,
    ));
    goat.mob_entity.persistence_required.store(true, Relaxed);
    let mut jumped = false;
    for _ in 0..80 {
        tick(&fixture, goat.as_ref());
        jumped |= goat.mob_entity.living_entity.jumping.load(Relaxed);
    }
    assert!(jumped);
    assert!(
        goat.get_entity().pos.load().y > 62.0,
        "goat sank: {:?}",
        goat.get_entity().pos.load()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn immobile_creaking_cannot_swim() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let creaking = CreakingEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::CREAKING,
    ));
    let mut goal = crate::entity::ai::goal::swim::SwimGoal::default().gated_by(|mob| {
        mob.cast_any()
            .downcast_ref::<CreakingEntity>()
            .is_some_and(CreakingEntity::can_move)
    });
    creaking.get_entity().touching_water.store(true, Relaxed);
    creaking.get_entity().water_height.store(2.0);
    assert!(goal.can_start(creaking.as_ref()));
    creaking.set_can_move(false);
    assert!(!goal.should_continue(creaking.as_ref()));
    assert!(!goal.can_start(creaking.as_ref()));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "flaky under parallel test load (passes alone); it shares the fixture world age, see briefs/report-opus-movement-2.md item 7"]
async fn vex_charges_target_over_full_ticks() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    floor(&world);
    let target = fixture.player("VexTarget");
    target.get_entity().set_pos(Vector3::new(12.5, 64.0, 4.5));
    let vex = VexEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 64.0, 4.5),
        &EntityType::VEX,
    ));
    vex.mob_entity.persistence_required.store(true, Relaxed);
    fixture.world().level_time.lock().unwrap().world_age = 20;
    vex.set_mob_target(Some(target.clone()));
    let initial = vex
        .get_entity()
        .pos
        .load()
        .squared_distance_to_vec(&target.get_entity().pos.load());
    let mut closest = initial;
    for _ in 0..120 {
        tick(&fixture, vex.as_ref());
        closest = closest.min(
            vex.get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&target.get_entity().pos.load()),
        );
    }
    assert!(
        closest < 4.0,
        "vex never approached its target: distance squared {closest}"
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn airborne_blaze_approaches_using_its_live_follow_range() {
    use crate::entity::mob::blaze::BlazeEntity;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    floor(&world);
    let target = fixture.player("BlazeTarget");
    target.get_entity().set_pos(Vector3::new(12.5, 64.0, 4.5));
    let blaze = BlazeEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 64.0, 4.5),
        &EntityType::BLAZE,
    ));
    blaze.entity.persistence_required.store(true, Relaxed);
    blaze
        .entity
        .living_entity
        .set_attribute_base(&pumpkin_data::attributes::Attributes::FOLLOW_RANGE, 1.0);
    blaze.set_mob_target(Some(target));
    for _ in 0..40 {
        tick(&fixture, blaze.as_ref());
    }
    assert!(blaze.get_entity().pos.load().x > 5.0);
    crate::server::fixture_lifecycle::finish().await;
}

struct TickOrderMob {
    mob: crate::entity::mob::MobEntity,
    observed_water: std::sync::atomic::AtomicBool,
    observed_input: crossbeam::atomic::AtomicCell<f64>,
}
impl Mob for TickOrderMob {
    fn get_mob_entity(&self) -> &crate::entity::mob::MobEntity {
        &self.mob
    }
    fn custom_server_ai_step(&self, _caller: &dyn EntityBase) {
        self.observed_water
            .store(self.get_entity().touching_water.load(Relaxed), Relaxed);
        self.mob.move_control.lock().unwrap().strafe(0.5, 0.5);
    }
    fn custom_travel(&self, _caller: &dyn EntityBase) -> bool {
        self.observed_input
            .store(self.mob.living_entity.movement_input.load().z);
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_reads_current_fluids_and_travel_reads_undecayed_control_input() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = floor(&world);
    chunk.set_block_absolute_y(4, 60, 4, Block::WATER.default_state.id);
    let mob = TickOrderMob {
        mob: crate::entity::mob::MobEntity::new(Entity::new(
            world,
            Vector3::new(4.5, 60.0, 4.5),
            &EntityType::COW,
        )),
        observed_water: std::sync::atomic::AtomicBool::new(false),
        observed_input: crossbeam::atomic::AtomicCell::new(0.0),
    };
    mob.mob.persistence_required.store(true, Relaxed);
    tick(&fixture, &mob);
    assert!(mob.observed_water.load(Relaxed));
    // The water probe forces MoveControl's forward-only input to exactly 1.0.
    assert_eq!(mob.observed_input.load(), 1.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn melee_goal_reaches_target_around_a_wall_over_full_ticks() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = floor(&world);
    for z in 0..9 {
        for y in 60..=62 {
            chunk.set_block_absolute_y(8, y, z, Block::STONE.default_state.id);
        }
    }
    let target = fixture.player("PathTarget");
    target.get_entity().set_pos(Vector3::new(12.5, 60.0, 4.5));
    let zombie = fixture.mob(&EntityType::ZOMBIE);
    zombie.get_entity().set_pos(Vector3::new(4.5, 60.0, 4.5));
    let mob = zombie.get_mob().unwrap();
    let data = mob.get_mob_entity();
    data.persistence_required.store(true, Relaxed);
    data.clear_ai_goals(mob);
    data.goals_selector
        .lock()
        .unwrap()
        .add_goal(1, Box::new(MeleeAttackGoal::new(1.0, true)));
    mob.set_mob_target(Some(target.clone()));
    let mut nodes = 0;
    let mut went_around = false;
    for _ in 0..400 {
        tick(&fixture, zombie.as_ref());
        if let Some(path) = data.navigator.lock().unwrap().get_path() {
            nodes = nodes.max(path.get_node_count());
        }
        went_around |= zombie.get_entity().pos.load().z >= 9.0;
        if data.is_in_attack_range(target.as_ref()) {
            break;
        }
    }
    assert!(nodes >= 6, "no multi-node path: {nodes}");
    assert!(went_around, "zombie did not go around the wall");
    assert!(
        data.is_in_attack_range(target.as_ref()),
        "zombie stopped at {:?}",
        zombie.get_entity().pos.load()
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn turtle_spawn_home_and_egg_keys_survive_save() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let turtle = TurtleEntity::new(Entity::new(
        world.clone(),
        Vector3::new(40.5, 63.0, -7.5),
        &EntityType::TURTLE,
    ));
    // Turtle.finalizeSpawn must use the final spawn position, not its constructor position.
    turtle.get_entity().set_pos(Vector3::new(45.5, 64.0, -10.5));
    let view = crate::world::spawn_view::SpawnView::live(&world);
    turtle.finalize_spawn(&world, &view, None);
    let mut saved = NbtCompound::new();
    turtle.mob_write_nbt(&mut saved);
    assert_eq!(
        saved.get_int_array("home_pos"),
        Some([45, 64, -11].as_slice())
    );
    for key in ["HasEgg", "has_egg"] {
        let mut legacy = NbtCompound::new();
        legacy.put_bool(key, true);
        turtle.mob_read_nbt(&legacy);
        saved = NbtCompound::new();
        turtle.mob_write_nbt(&mut saved);
        assert_eq!(saved.get_bool("has_egg"), Some(true));
        assert!(saved.get_bool("HasEgg").is_none());
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn bat_flight_does_not_accumulate_fall_distance() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let bat = BatEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 64.0, 4.5),
        &EntityType::BAT,
    ));
    let living = &bat.mob_entity.living_entity;
    living.fall_distance.store(5.0);
    living.fall(bat.as_ref(), -1.0, false, false);
    assert_eq!(living.fall_distance.load(), 0.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn wall_climber_stop_keeps_fallback_but_reports_path_done() {
    use crate::entity::ai::pathfinder::{PathNavigationTrait, WallClimberNavigation};
    let mut navigation = WallClimberNavigation::new();
    navigation.path_to_position = Some(BlockPos::new(12, 60, 4));
    navigation.stop();
    assert_eq!(navigation.path_to_position, Some(BlockPos::new(12, 60, 4)));
    assert!(navigation.is_done());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn queued_coordinate_path_does_not_reuse_previous_reach_range() {
    use crate::entity::ai::pathfinder::{GroundPathNavigation, NavigatorGoal, PathNavigationTrait};
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    floor(&world);
    let goat = GoatEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::GOAT,
    ));
    goat.get_entity().on_ground.store(true, Relaxed);
    let mut navigation = GroundPathNavigation::new();
    let mob = &goat.mob_entity;
    assert!(
        navigation
            .create_path(mob, Vector3::new(12.5, 60.0, 4.5), 6)
            .is_some()
    );
    navigation.set_progress(NavigatorGoal::new(
        goat.get_entity().pos.load(),
        Vector3::new(12.5, 60.0, 4.5),
        1.0,
    ));
    navigation.tick(mob, goat.as_ref());
    let path = navigation.get_path().unwrap();
    assert_eq!(path.get_end_node().unwrap().pos, BlockPos::new(11, 60, 4));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn flying_destination_tests_its_own_collision_face() {
    use crate::entity::ai::pathfinder::{FlyingPathNavigation, PathNavigationTrait};
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let chunk = floor(&world);
    let allay = crate::entity::passive::allay::AllayEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 62.0, 4.5),
        &EntityType::ALLAY,
    ));
    let navigation = FlyingPathNavigation::new();
    let pos = BlockPos::new(4, 60, 4);
    assert!(!navigation.is_stable_destination(&world, &pos, allay.as_ref()));
    chunk.set_block_absolute_y(4, 60, 4, Block::STONE.default_state.id);
    assert!(navigation.is_stable_destination(&world, &pos, allay.as_ref()));
    chunk.set_block_absolute_y(4, 60, 4, Block::OAK_SLAB.default_state.id);
    assert!(!navigation.is_stable_destination(&world, &pos, allay.as_ref()));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phantom_spawn_anchor_drives_idle_flight_above_the_spawn() {
    use crate::entity::mob::phantom::PhantomEntity;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    floor(&world);
    let phantom = PhantomEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 64.0, 4.5),
        &EntityType::PHANTOM,
    ));
    phantom.mob_entity.persistence_required.store(true, Relaxed);
    phantom.get_entity().set_pos(Vector3::new(5.5, 70.0, 4.5));
    phantom.finalize_spawn(
        &world,
        &crate::world::spawn_view::SpawnView::live(&world),
        None,
    );
    tick(&fixture, phantom.as_ref());
    assert_eq!(phantom.anchor_point.load(), Some(BlockPos::new(5, 75, 4)));
    assert!(phantom.move_target_point.load().y >= 67.0);
    crate::server::fixture_lifecycle::finish().await;
}
