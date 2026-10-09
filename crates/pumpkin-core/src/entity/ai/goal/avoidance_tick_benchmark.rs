use crate::{
    entity::{
        Entity, EntityBase, ai::goal::avoid_entity::AvoidEntityGoal, living::LivingEntity,
        passive::villager::VillagerEntity,
    },
    server::combat_test_support,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::{hint::black_box, sync::Arc, time::Instant};

// Nine representative threat queries use fixed ranges for comparison, not vanilla Brain ticks.
const SCANS: [(&EntityType, f64); 9] = [
    (&EntityType::ZOMBIE, 8.0),
    (&EntityType::ZOMBIE_VILLAGER, 8.0),
    (&EntityType::HUSK, 8.0),
    (&EntityType::DROWNED, 8.0),
    (&EntityType::PILLAGER, 8.0),
    (&EntityType::VINDICATOR, 8.0),
    (&EntityType::EVOKER, 8.0),
    (&EntityType::RAVAGER, 8.0),
    (&EntityType::VEX, 12.0),
];

#[tokio::test]
#[ignore = "release avoid-goal threat scan benchmark; no server startup"]
async fn avoidance_tick_benchmark() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let villagers: Vec<_> = (0..50)
        .map(|i| {
            VillagerEntity::new(Entity::new(
                world.clone(),
                Vector3::new(f64::from(i % 10) * 4.0, 64.0, f64::from(i / 10) * 4.0),
                &EntityType::VILLAGER,
            ))
        })
        .collect();
    let mut entities = village_entities(&world, &villagers);
    assert_eq!(entities.len(), 1000);
    world.entities.store(Arc::new(entities.clone()));
    let boxes: Vec<_> = villagers
        .iter()
        .flat_map(|villager| {
            SCANS.iter().map(move |(_, range)| {
                villager
                    .get_entity()
                    .bounding_box
                    .load()
                    .expand(*range, 3.0, *range)
            })
        })
        .collect();
    // Floor: the same 450 box queries, with no type filtering, target tests or handle copies.
    measure("floor", || {
        for aabb in &boxes {
            let entities = world.entities.load();
            for entity in entities.iter() {
                black_box(entity.get_entity().bounding_box.load().intersects(aabb));
            }
        }
    });
    // AvoidEntityGoal.canUse's threat lookup only: no escape sampling, navigation or full tick.
    let scan = || {
        for villager in &villagers {
            for (entity_type, range) in SCANS {
                let _ = black_box(AvoidEntityGoal::find_threat(
                    villager.as_ref(),
                    range,
                    |target| target.get_entity().entity_type == entity_type,
                ));
            }
        }
    };
    assert!(
        AvoidEntityGoal::find_threat(villagers[0].as_ref(), 8.0, |target| {
            target.get_entity().entity_type == &EntityType::ZOMBIE
        })
        .is_none()
    );
    measure("threat_free", scan);
    let zombie = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(2.0, 64.0, 0.0),
        &EntityType::ZOMBIE,
    )));
    entities[999] = zombie.clone();
    world.entities.store(Arc::new(entities));
    assert_eq!(
        AvoidEntityGoal::find_threat(villagers[0].as_ref(), 8.0, |target| {
            target.get_entity().entity_type == &EntityType::ZOMBIE
        })
        .unwrap()
        .get_entity()
        .entity_id,
        zombie.get_entity().entity_id
    );
    measure("one_threat", scan);
}

#[expect(
    clippy::print_stdout,
    reason = "An explicitly requested ignored benchmark reports its samples"
)]
fn measure(scenario: &str, run_scan: impl Fn()) {
    for _ in 0..32 {
        run_scan();
    }
    for run in 1..=3 {
        let mut samples = Vec::with_capacity(101);
        for _ in 0..101 {
            let start = Instant::now();
            run_scan();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "BENCH scenario={scenario} run={run} median_ms={:.6} min_ms={:.6} max_ms={:.6}",
            samples[50], samples[0], samples[100]
        );
    }
}

fn village_entities(
    world: &Arc<crate::world::World>,
    villagers: &[Arc<VillagerEntity>],
) -> Vec<Arc<dyn EntityBase>> {
    let mut entities: Vec<Arc<dyn EntityBase>> = villagers
        .iter()
        .map(|mob| mob.clone() as Arc<dyn EntityBase>)
        .collect();
    for i in 0..950 {
        entities.push(Arc::new(Entity::new(
            world.clone(),
            Vector3::new(
                f64::from(i % 100 - 50) * 2.0,
                64.0,
                f64::from(i / 100) * 8.0 - 40.0,
            ),
            &EntityType::COW,
        )));
    }
    entities
}
