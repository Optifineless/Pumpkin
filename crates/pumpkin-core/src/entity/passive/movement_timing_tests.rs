//! Repeatable, opt-in timing of `Mob.serverAiStep`, including `MoveControl`'s strafe probe.
use crate::entity::{
    Entity, EntityBase,
    living::{LivingEntity, test_support::armor_test_world},
    passive::cow::CowEntity,
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::{sync::Arc, time::Instant};

#[tokio::test]
#[ignore = "Manual timing harness; run alone with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "Opt-in benchmark reports wall time")]
async fn server_ai_step_timing() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let chunk = ChunkData::empty_sync(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_absolute_y(x, 59, z, Block::STONE.default_state.id);
        }
    }
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let cow = CowEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::COW,
    ));
    let mob = cow.get_mob().unwrap().get_mob_entity();
    mob.clear_ai_goals(cow.as_ref());
    let filler: Arc<dyn EntityBase> = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(10.5, 60.0, 10.5),
        &EntityType::COW,
    )));
    for population in [0, 2048] {
        // Put the tested mob last to exercise the old self-lookup's full scan.
        let mut entities = vec![filler.clone(); population];
        entities.push(cow.clone());
        world.entities.store(Arc::new(entities));
        for strafe in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                for _ in 0..10_000 {
                    if strafe {
                        mob.move_control.lock().unwrap().strafe(0.5, 0.5);
                    }
                    mob.server_ai_step(cow.as_ref(), cow.as_ref());
                }
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "server_ai_step: population={population}, strafe={strafe}, 10000 ticks: median={:.3} ms, samples={samples:?}",
                samples[2]
            );
        }
    }
    world.entities.store(Arc::new(Vec::new()));
}
