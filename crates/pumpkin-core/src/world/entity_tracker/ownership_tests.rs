#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]
use super::*;
use crate::{
    entity::living::LivingEntity,
    server::combat_test_support::{server, world},
};
use pumpkin_data::damage::DamageType;
use std::time::Instant;

use crate::{
    entity::item::ItemEntity,
    plugin::{BoxFuture, EventHandler, EventPriority, entity::item_spawn::ItemSpawnEvent},
    server::Server,
};
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering::SeqCst},
};

struct DeathDrop {
    victim: Arc<dyn EntityBase>,
    drop: Arc<dyn EntityBase>,
    barrier: Arc<Barrier>,
    inserted: AtomicBool,
}
impl EventHandler<ItemSpawnEvent> for DeathDrop {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if !self.inserted.swap(true, SeqCst) {
            let living = self.victim.get_living_entity().unwrap();
            living.with_damage_owned(|| {
                self.barrier.wait();
                assert!(living.wait_until_damage_contended());
                let world = self.victim.get_entity().world.load();
                // Fail without hanging the test process if iteration retains the read lock.
                assert!(
                    world
                        .entity_tracker
                        .entity_map
                        .try_entry(self.drop.get_entity().entity_id)
                        .is_some(),
                    "death loot insertion would deadlock on the tracker's shard guard"
                );
                world.add_entity_silent(self.drop.clone());
            });
        }
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_death_loot_inserts_into_the_tracker_iteration_shard() {
    use pumpkin_data::{item::Item, item_stack::ItemStack};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = crate::entity::r#type::from_type(
        &EntityType::PIG,
        Vector3::default(),
        &world,
        Uuid::new_v4(),
    );
    world.add_entity_silent(victim.clone());
    let tracker = &world.entity_tracker;
    // A nonblocking write probe while holding the victim's read guard finds a guaranteed collision.
    let guard = tracker
        .entity_map
        .get(&victim.get_entity().entity_id)
        .unwrap();
    let collision = (-100_000..0)
        .find(|id| tracker.entity_map.try_entry(*id).is_none())
        .unwrap();
    drop(guard);
    let drop = Arc::new(ItemEntity::new(
        Entity::from_uuid_with_id(
            collision,
            Uuid::new_v4(),
            world.clone(),
            Vector3::default(),
            &EntityType::ITEM,
        ),
        ItemStack::new(1, &Item::PORKCHOP),
    )) as Arc<dyn EntityBase>;
    let barrier = Arc::new(Barrier::new(2));
    let handler = Arc::new(DeathDrop {
        victim: victim.clone(),
        drop,
        barrier: barrier.clone(),
        inserted: AtomicBool::new(false),
    });
    server.plugin_manager.register::<ItemSpawnEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            tracker.update_all(&world);
        });
        assert!(victim.damage(victim.as_ref(), 100.0, DamageType::GENERIC_KILL));
    });
    assert!(handler.inserted.load(SeqCst));
    assert!(tracker.has_entity_with_id(collision));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ownership_review_tracking_reuses_the_tracked_owner_without_nested_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let living = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    )));
    let tracked = TrackedEntity::new(living.clone(), 8, 3, false);
    world.entities.store(Arc::new(vec![living.clone()]));
    // ServerEntity.sendChanges already owns its tracked entity throughout motion delivery.
    let baseline = living.damage_entry_count();
    for _ in 0..100 {
        tracked.send_changes(&world);
    }
    assert_eq!(living.damage_entry_count() - baseline, 100);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification4_tracking_reuses_snapshot_without_retaining_entities() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let living = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ))) as Arc<dyn EntityBase>;
    world.entity_tracker.add_entity(&living, &world);
    world.entity_tracker.update_all(&world);
    let address = UPDATE_SNAPSHOT.with(|buffer| {
        let buffer = buffer.borrow();
        assert!(buffer.is_empty());
        assert!(buffer.capacity() > 0);
        buffer.as_ptr()
    });
    world.entity_tracker.update_all(&world);
    UPDATE_SNAPSHOT.with(|buffer| {
        let buffer = buffer.borrow();
        assert!(buffer.is_empty());
        assert_eq!(address, buffer.as_ptr());
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Manual debug-profile tracking and damage benchmark"]
#[expect(
    clippy::print_stdout,
    reason = "Manual benchmark reports measured timings"
)]
async fn ownership_review_tracking_and_hits_benchmark() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mobs: Vec<Arc<dyn EntityBase>> = (0..1000)
        .map(|_| {
            Arc::new(LivingEntity::new(Entity::new(
                world.clone(),
                Vector3::default(),
                &EntityType::ZOMBIE,
            ))) as Arc<dyn EntityBase>
        })
        .collect();
    world.entities.store(Arc::new(mobs.clone()));
    for mob in &mobs {
        world.entity_tracker.add_entity(mob, &world);
    }
    let start = Instant::now();
    for _ in 0..100 {
        world.entity_tracker.update_all(&world);
    }
    let tracking = start.elapsed();
    let victim = &mobs[0];
    let living = victim.get_living_entity().unwrap();
    let start = Instant::now();
    for _ in 0..10_000 {
        living.set_health(20.0);
        living.hurt_cooldown.store(0, Relaxed);
        assert!(victim.damage(victim.as_ref(), 1.0, DamageType::GENERIC));
    }
    println!(
        "debug benchmark: tracking_1000x100={tracking:?} hits_10000={:?}",
        start.elapsed()
    );
}
