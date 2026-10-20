use super::happy_ghast_test_support::*;
use crate::{
    command::CommandSender,
    entity::{EntityBase, ageable::AgeableMob},
    plugin::{
        EventHandler, EventPriority,
        api::events::entity::{
            entity_regain_health::EntityRegainHealthEvent, entity_spawn::EntitySpawnEvent,
        },
    },
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering::Relaxed};

struct CancelSpawn;
impl EventHandler<EntitySpawnEvent> for CancelSpawn {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_real_spawn_ticks_adult_flight() {
    let fixture = Fixture::new(true).await;
    let command = fixture.summon("8.5 74 8.5", "");
    let egg = fixture.egg(BlockPos::new(-8, 74, 8));
    for entity in [&command, &egg] {
        draws(entity, &[0.75, 0.625, 0.75]);
    }
    let starts = [command.get_entity().pos.load(), egg.get_entity().pos.load()];
    fixture.tick(1);
    let first_delta = command.get_entity().pos.load() - starts[0];
    assert!(
        first_delta.x > 0.0 && first_delta.z > 0.0,
        "adult installation did not move toward its real 3D destination: {first_delta:?}"
    );
    assert!(
        (command.get_entity().velocity.load().y - 0.025_277_778_506_278_99).abs() < 1.0e-9,
        "real flying travel did not apply vanilla air drag"
    );
    fixture.tick(15);
    for (entity, start) in [(&command, starts[0]), (&egg, starts[1])] {
        assert_flying(entity, start);
    }
    drop(fixture.close().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_legacy_and_admission_controls() {
    let fixture = Fixture::new(true).await;
    let no_ai = fixture.summon("-8.5 90 -8.5", "{NoAI:1b}");
    let baby = fixture.summon("8.5 90 -8.5", "{Age:-24000}");
    fixture.tick(16);
    assert_eq!(
        no_ai.get_entity().pos.load(),
        Vector3::new(-8.5, 90.0, -8.5)
    );
    assert!(
        baby.get_entity().pos.load().y < 90.0,
        "baby must retain legacy gravity"
    );
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(Arc::new(CancelSpawn), EventPriority::Normal, true);
    let count = fixture.world.entities.load().len();
    let mut source = CommandSender::Console
        .into_source(&fixture.server)
        .with_world(fixture.world.clone());
    source.silent = true;
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input("summon minecraft:happy_ghast 0 90 0", &source)
        .unwrap();
    fixture.tick(2);
    assert_eq!(
        fixture.world.entities.load().len(),
        count,
        "cancelled spawn was admitted"
    );
    drop(fixture.close().await);
    let disabled = Fixture::new(false).await;
    let entity = disabled.summon("8.5 90 8.5", "");
    disabled.tick(8);
    assert!(entity.get_entity().pos.load().y < 90.0);
    assert!(nbt(&entity).get_int("home_radius").is_none());
    drop(disabled.close().await);
}

async fn careful_case(
    block: Option<&'static Block>,
    pos: BlockPos,
    matching: Option<&'static Block>,
    reject: bool,
) -> bool {
    let fixture = Fixture::new(true).await;
    if let Some(block) = block {
        fixture
            .world
            .set_block_state(&pos, block.default_state.id, BlockFlags::FORCE_STATE);
    }
    let entity = fixture.summon("8.0 74 8.0", "");
    assert_eq!(entity.get_entity().pos.load(), Vector3::new(8.0, 74.0, 8.0));
    if let Some(block) = matching {
        // Actual current-fluid occupancy, with dry destination margin at X=20.
        for x in 6..=9 {
            for y in 74..=77 {
                for z in 6..=9 {
                    fixture.world.set_block_state(
                        &BlockPos::new(x, y, z),
                        block.default_state.id,
                        BlockFlags::FORCE_STATE,
                    );
                }
            }
        }
    }
    wanted(&entity, Vector3::new(20.0, 74.0, 8.0));
    fixture.tick(1);
    let velocity = entity.get_entity().velocity.load();
    let expected = if reject {
        velocity.x == 0.0
    } else {
        velocity.x > 0.0
    };
    assert!(
        !ghast(&entity)
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .is_in_progress()
    );
    drop(fixture.close().await);
    expected
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_careful_flight_uses_whole_body() {
    let mut failures = Vec::new();
    for block in [
        &Block::WITHER_ROSE,
        &Block::IRON_BARS,
        &Block::WATER,
        &Block::LAVA,
    ] {
        if !careful_case(Some(block), BlockPos::new(14, 75, 8), None, true).await {
            failures.push(block.name);
        }
    }
    // X=23 is exactly floor(destination body's expanded maximum), a closed vanilla boundary.
    if !careful_case(
        Some(&Block::WITHER_ROSE),
        BlockPos::new(23, 75, 8),
        None,
        true,
    )
    .await
    {
        failures.push("closed destination margin");
    }
    assert!(
        failures.is_empty(),
        "careful control accelerated through rejected paths: {failures:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_clear_route_hostile_and_bee_controls() {
    assert!(
        careful_case(None, BlockPos::new(14, 75, 8), None, false).await,
        "clear route rejected"
    );
    for block in [&Block::IRON_BARS, &Block::WATER, &Block::LAVA] {
        assert!(
            careful_case(Some(block), BlockPos::new(14, 75, 10), None, false).await,
            "nonintersecting {} rejected",
            block.name
        );
    }
    for block in [&Block::WATER, &Block::LAVA] {
        assert!(
            careful_case(Some(block), BlockPos::new(14, 75, 8), Some(block), false).await,
            "matching {} rejected",
            block.name
        );
    }
    let fixture = Fixture::new(true).await;
    let hostile = crate::entity::r#type::from_type(
        &EntityType::GHAST,
        Vector3::new(8.0, 74.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    assert!(fixture.world.spawn_entity(hostile.clone()));
    hostile
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .move_control
        .lock()
        .unwrap()
        .set_wanted_position(20.0, 74.0, 8.0, 1.0);
    fixture.world.set_block_state(
        &BlockPos::new(14, 75, 8),
        Block::WITHER_ROSE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let bee = crate::entity::r#type::from_type(
        &EntityType::BEE,
        Vector3::new(-8.0, 74.0, 8.0),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    assert!(fixture.world.spawn_entity(bee.clone()));
    fixture.tick(1);
    assert!(
        hostile.get_entity().velocity.load().x > 0.0,
        "hostile default was changed"
    );
    assert!(nbt(&bee).get_int("home_radius").is_none());
    assert!(!bee.get_entity().is_removed());
    drop(fixture.close().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_loaded_timeout_stops_adult_control() {
    let fixture = Fixture::new(true).await;
    let entity = fixture.summon("8.5 74 8.5", "{Age:80,still_timeout:6}");
    align_existing_selector_phase(&entity);
    entity.get_entity().yaw.store(116.0);
    entity
        .get_entity()
        .velocity
        .store(Vector3::new(0.3, 0.2, 0.4));
    ghast(&entity)
        .mob_entity
        .living_entity
        .movement_input
        .store(Vector3::new(0.1, 0.2, 0.3));
    draws(&entity, &[0.75, 0.625, 0.75].repeat(100));
    for _ in 0..5 {
        fixture.tick(1);
        assert!(ghast(&entity).server_still_timeout.load(Relaxed) > 0);
        assert_eq!(
            entity.get_entity().velocity.load(),
            Vector3::new(0.0, 0.0, 0.0),
            "positive timeout allowed movement"
        );
        assert_eq!(
            ghast(&entity)
                .mob_entity
                .living_entity
                .movement_input
                .load(),
            Vector3::new(0.0, 0.0, 0.0)
        );
        assert_eq!(entity.get_entity().yaw.load(), 90.0);
        assert_eq!(entity.get_entity().head_yaw.load(), 90.0);
    }
    let start = entity.get_entity().pos.load();
    fixture.tick(12);
    assert_flying(&entity, start);
    entity.get_entity().remove();
    let removed_pos = entity.get_entity().pos.load();
    fixture.tick(12);
    assert_eq!(entity.get_entity().pos.load(), removed_pos);
    drop(fixture.close().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_wet_timeout_gates_float_goal() {
    for block in [&Block::WATER, &Block::LAVA] {
        let fixture = Fixture::new(true).await;
        for x in 6..=10 {
            for y in 74..=78 {
                for z in 6..=10 {
                    fixture.world.set_block_state(
                        &BlockPos::new(x, y, z),
                        block.default_state.id,
                        BlockFlags::FORCE_STATE,
                    );
                }
            }
        }
        let entity = fixture.summon("8.5 74 8.5", "{Age:80,still_timeout:6}");
        align_existing_selector_phase(&entity);
        draws(&entity, &[0.75, 0.625, 0.75].repeat(100));
        for _ in 0..5 {
            fixture.tick(1);
            assert!(ghast(&entity).is_on_still_timeout());
            assert_eq!(
                entity.get_entity().velocity.load(),
                Vector3::new(0.0, 0.0, 0.0),
                "FloatGoal overrode positive timeout in {block:?}"
            );
            assert_eq!(
                ghast(&entity)
                    .mob_entity
                    .living_entity
                    .movement_input
                    .load(),
                Vector3::new(0.0, 0.0, 0.0)
            );
        }
        let start = entity.get_entity().pos.load();
        fixture.tick(16);
        assert_flying(&entity, start);
        drop(fixture.close().await);
    }
}

fn align_existing_selector_phase(entity: &Arc<dyn EntityBase>) {
    // Existing shared biological/elapsed age pins positive-age selector parity.
    // Load a valid legacy cooldown age in the full-selector phase; do not change that shared owner.
    let mut legacy = nbt(entity);
    legacy.put_int("Age", 80 + entity.get_entity().entity_id.rem_euclid(2));
    entity.read_nbt_non_mut(&legacy);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_timeout_and_mode_controls() {
    let fixture = Fixture::new(true).await;
    for nbt in ["{}", "{still_timeout:0}"] {
        let entity = fixture.summon("-8.5 74 8.5", nbt);
        draws(&entity, &[0.75, 0.625, 0.75]);
        fixture.tick(12);
        assert_flying(&entity, Vector3::new(-8.5, 74.0, 8.5));
        entity.get_entity().remove();
    }
    let baby = fixture.summon("8.5 90 8.5", "{Age:-24000,still_timeout:100}");
    fixture.tick(2);
    assert!(baby.get_entity().pos.load().y < 90.0);
    // Exercise the existing age setter, then normal world AI, without a universal age callback.
    ghast(&baby).set_age(0);
    ghast(&baby).set_server_still_timeout(0);
    draws(&baby, &[0.75, 0.625, 0.75]);
    let start = baby.get_entity().pos.load();
    fixture.tick(12);
    assert!((baby.get_entity().pos.load() - start).length() > 0.05);
    assert_eq!(ghast(&baby).floating_goal_count(), 1);
    let mut data = nbt(&baby);
    data.put_int("Age", -24000);
    baby.read_nbt_non_mut(&data);
    let start = baby.get_entity().pos.load();
    fixture.tick(12);
    assert!(baby.get_entity().pos.load().y < start.y);
    assert_eq!(ghast(&baby).floating_goal_count(), 0);
    ghast(&baby).set_age(0);
    draws(&baby, &[0.75, 0.625, 0.75]);
    let start = baby.get_entity().pos.load();
    fixture.tick(32);
    assert!((baby.get_entity().pos.load() - start).length() > 0.05);
    assert_eq!(ghast(&baby).floating_goal_count(), 1);
    drop(fixture.close().await);
    let fixture = Fixture::new(false).await;
    let disabled = fixture.summon("8.5 90 8.5", "{still_timeout:100}");
    fixture.tick(2);
    assert!(
        disabled.get_entity().pos.load().y < 90.0,
        "disabled legacy adult was stopped by new control"
    );
    assert!(nbt(&disabled).get_int("home_radius").is_none());
    drop(fixture.close().await);
}

struct ChangeAgeOnHeal(Arc<dyn EntityBase>, i32);
impl EventHandler<EntityRegainHealthEvent> for ChangeAgeOnHeal {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityRegainHealthEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.0.get_entity().entity_id {
            ghast(&self.0).set_age(self.1);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_happy_ghast_s1_callback_age_change_reconciles_before_ai() {
    let fixture = Fixture::new(true).await;
    // Advance the existing clock owner to the normal heal boundary without replacing its state.
    for _ in 0..599 {
        fixture.world.tick_environment();
    }
    let adult = fixture.summon("8.5 90 8.5", "{}");
    let baby = fixture.summon("-8.5 74 8.5", "{Age:-100}");
    assert_eq!(fixture.world.get_world_age(), 599);
    for (entity, age) in [(&adult, -24000), (&baby, 0)] {
        entity.get_living_entity().unwrap().set_health(10.0);
        fixture
            .server
            .plugin_manager
            .register::<EntityRegainHealthEvent, _>(
                Arc::new(ChangeAgeOnHeal(entity.clone(), age)),
                EventPriority::Normal,
                true,
            );
        draws(entity, &[0.75, 0.625, 0.75]);
    }
    wanted(&adult, Vector3::new(20.5, 94.0, 16.5));
    fixture.tick(1);
    assert!(ghast(&adult).get_age() < 0);
    assert_eq!(
        ghast(&adult).floating_goal_count(),
        0,
        "heal callback left adult goals on a baby"
    );
    assert_eq!(
        adult.get_entity().velocity.load().x,
        0.0,
        "stale adult controller accelerated a baby"
    );
    assert!(ghast(&baby).get_age() >= 0);
    assert_eq!(
        ghast(&baby).floating_goal_count(),
        1,
        "heal callback left legacy goals on an adult"
    );
    assert!(
        baby.get_entity().velocity.load().x > 0.0,
        "new adult missed normal AI steering"
    );
    drop(fixture.close().await);
}
