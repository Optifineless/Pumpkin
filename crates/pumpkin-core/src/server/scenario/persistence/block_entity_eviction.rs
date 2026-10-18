use super::block_entity_support::{CHEST, wait_for_crossbeam_barrier};
use super::*;
use pumpkin_world::chunk::io::Dirtiable;
use std::sync::atomic::Ordering;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_image_is_dirty_before_real_terrain_eviction() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "DirtyBeforeEvict").await;
        let world = player.player.world();
        let position = CHEST.chunk_position();
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let chest = Arc::new(ChestBlockEntity::new(CHEST));
        world.add_block_entity(chest.clone());
        chest.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
        let entity: Arc<dyn crate::block::entities::BlockEntity> = chest.clone();
        world.update_block_entity(&entity);
        // Drain registration/update migrations before preserving this exact save image below.
        scenario.ticks(1).await;
        let chunk = world.level.loaded_chunks.get(&position).unwrap().clone();
        scenario
            .drive(
                "clean chest baseline",
                world.level.write_chunks(vec![(position, chunk.clone())]),
            )
            .await;
        assert!(!chunk.is_dirty(), "baseline chest chunk was not clean");
        chest.set_stack(0, ItemStack::new(7, &Item::EMERALD));
        assert!(
            !chunk.is_dirty(),
            "inventory edit dirtied terrain before the save publication"
        );

        let (published, wait_for_publication) = crossbeam::channel::bounded(1);
        let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
        *world.persistence_test_hooks.publication.lock().unwrap() = Some(Arc::new(move || {
            published.send(()).unwrap();
            wait_for_crossbeam_barrier(&wait_for_resume, "terrain publication resume");
        }));
        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_crossbeam_barrier(&wait_for_publication, "terrain publication");
        })
        .await
        .unwrap();

        // Keep chunk retirement and shutdown from repairing this publication with a later capture.
        world
            .block_entities
            .get_mut(&position)
            .unwrap()
            .remove(&CHEST);
        scenario.command("tp DirtyBeforeEvict 163.5 64 3.5");
        player.loaded(&scenario.server);
        scenario
            .until("terrain eviction watcher departure", || {
                player.is_idle() && scenario.server.tasks.is_empty()
            })
            .await;
        scenario.ticks(1).await;
        assert!(!world.level.is_chunk_watched(&position));
        // Wake the real schedule's unload queue rather than waiting for its batching timer.
        scenario
            .until("published terrain eviction", || {
                world.level.should_unload.store(true, Ordering::Relaxed);
                world.level.level_channel.notify();
                !world.level.is_chunk_loaded(&position)
            })
            .await;
        // A new scheduler load cannot finish until process_unload_queue has made its dirty
        // decision and its I/O owner has ordered the previous write before this read.
        let reloaded = scenario
            .drive(
                "terrain eviction I/O barrier",
                world.level.get_or_fetch_chunk(position, Clone::clone),
            )
            .await
            .unwrap();
        assert!(
            !Arc::ptr_eq(&chunk, &reloaded),
            "terrain eviction reused the old chunk incarnation"
        );
        resume.send(()).unwrap();
        save.await.unwrap();

        // Restart joins the actual terrain writer; the evicted Arc cannot be resubmitted by stop.
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadEvictedImage").await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&CHEST)
            .expect("evicted chest missing after disk reload")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(
            stack.item,
            &Item::EMERALD,
            "terrain eviction lost the new persistent image"
        );
        assert_eq!(stack.item_count, 7);
        scenario.stop().await;
    })
    .await;
}
