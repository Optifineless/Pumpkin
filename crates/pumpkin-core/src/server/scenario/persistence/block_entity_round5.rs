use super::block_entity_round5_support::{
    GatedBookshelfCapture, PausedRegistrationChest, PredecessorGateChest,
    wait_for_crossbeam_capture,
};
use super::block_entity_support::*;
use super::*;
use crate::block::entities::{
    BlockEntity, chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packet_update_cannot_replace_a_full_deferred_loot_image() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "FullChestSave").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let mut chest = ChestBlockEntity::new(CHEST);
        *chest
            .loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some("minecraft:chests/simple_dungeon".into());
        chest.loot_table_seed = 0x172;
        let paused = Arc::new(PausedChestCapture {
            chest: Arc::new(chest),
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        let entity: Arc<dyn BlockEntity> = paused.clone();
        world.add_block_entity(entity.clone());
        paused.pause.store(true, Ordering::Release);

        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "deferred loot save capture");
        })
        .await
        .unwrap();

        world.update_block_entity(&entity);
        resume_capture.send(()).unwrap();
        save.await.unwrap();

        let chunk = world
            .level
            .loaded_chunks
            .get(&CHEST.chunk_position())
            .unwrap()
            .clone();
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&CHEST)
            .cloned()
            .expect("overlapping update left no persistent chest image");
        assert_eq!(
            saved.get_string("LootTable"),
            Some("minecraft:chests/simple_dungeon"),
            "packet NBT replaced the deferred loot table in the save image"
        );
        assert_eq!(saved.get_long("LootTableSeed"), Some(0x172));

        world
            .block_entities
            .get_mut(&CHEST.chunk_position())
            .unwrap()
            .remove(&CHEST);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadFullChest").await;
        let loaded = player.player.world().get_block_entity(&CHEST).unwrap();
        assert!(loaded.has_loot_table());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn older_fork_packet_shaped_chest_image_loads_after_disk_reload() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "LegacyChestImage").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        );

        let chest = Arc::new(ChestBlockEntity::new(CHEST));
        chest.set_stack(0, ItemStack::new(4, &Item::EMERALD));
        let mut legacy = chest.chunk_data_nbt().unwrap();
        assert_eq!(
            legacy
                .get_list("Items")
                .map(<[pumpkin_nbt::tag::NbtTag]>::len),
            Some(1)
        );
        legacy.put_string("id", ChestBlockEntity::ID.to_owned());
        legacy.put_int("x", CHEST.0.x);
        legacy.put_int("y", CHEST.0.y);
        legacy.put_int("z", CHEST.0.z);
        world.add_block_entity_nbt(CHEST, &legacy);

        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "LoadLegacyChestImage").await;
        let world = player.player.world();
        let chunk = world
            .level
            .loaded_chunks
            .get(&CHEST.chunk_position())
            .unwrap()
            .clone();
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&CHEST)
            .cloned()
            .expect("legacy chest image disappeared after disk reload");
        assert_eq!(
            saved
                .get_list("Items")
                .map(<[pumpkin_nbt::tag::NbtTag]>::len),
            Some(1)
        );
        let inventory = world
            .get_block_entity(&CHEST)
            .expect("legacy chest image did not promote after disk reload")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::EMERALD);
        assert_eq!(stack.item_count, 4);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_incarnation_resets_an_older_published_stamp() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "IncarnationStamp").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let old = Arc::new(ChestBlockEntity::new(CHEST));
        old.set_stack(0, ItemStack::new(2, &Item::DIAMOND));
        let old_entity: Arc<dyn BlockEntity> = old.clone();
        world.add_block_entity(old_entity.clone());

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let replacement = Arc::new(ChestBlockEntity::new(CHEST));
        replacement.set_stack(0, ItemStack::new(5, &Item::EMERALD));
        let paused = Arc::new(PausedRegistrationChest {
            chest: replacement,
            captured,
            resume: resume_writer,
            pause: AtomicBool::new(true),
        });
        let add_world = world.clone();
        let add_entity = paused.clone();
        let add = std::thread::spawn(move || add_world.add_block_entity(add_entity));
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "replacement packet NBT");
        })
        .await
        .unwrap();

        old.set_stack(0, ItemStack::new(3, &Item::GOLD_INGOT));
        world.update_block_entity(&old_entity);
        resume_capture.send(()).unwrap();
        add.join().unwrap();

        let chunk = world
            .level
            .loaded_chunks
            .get(&CHEST.chunk_position())
            .unwrap()
            .clone();
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&CHEST)
            .cloned()
            .expect("replacement registration left no persistent image");
        let restored = ChestBlockEntity::from_nbt(&saved, CHEST);
        assert_eq!(restored.get_stack(0).item, &Item::EMERALD);
        assert_eq!(restored.get_stack(0).item_count, 5);

        // Preserve the registration image before stop captures the live replacement again.
        world
            .block_entities
            .get_mut(&CHEST.chunk_position())
            .unwrap()
            .remove(&CHEST);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadIncarnation").await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&CHEST)
            .expect("new chest incarnation missing after reload")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::EMERALD);
        assert_eq!(stack.item_count, 5);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_save_follows_replacements_until_the_current_image_is_saved() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "ReplacementChurn").await;
        let world = player.player.world();
        world.set_block_state(
            &BOOKSHELF,
            Block::CHISELED_BOOKSHELF.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let first = Arc::new(GatedBookshelfCapture::new(
            Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)),
            0,
            captured.clone(),
            resume_writer.clone(),
        ));
        let first_entity: Arc<dyn BlockEntity> = first.clone();
        world.add_block_entity(first_entity);
        first.pause.store(true, Ordering::Release);

        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        assert_eq!(
            wait_for_crossbeam_capture(&wait_for_capture, "first save incarnation").await,
            0
        );

        let second = Arc::new(GatedBookshelfCapture::new(
            Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)),
            1,
            captured.clone(),
            resume_writer.clone(),
        ));
        let second_entity: Arc<dyn BlockEntity> = second.clone();
        world.add_block_entity(second_entity);
        second.bookshelf.set_book(0, ItemStack::new(1, &Item::BOOK));
        second.pause.store(true, Ordering::Release);
        resume_capture.send(()).unwrap();
        assert_eq!(
            wait_for_crossbeam_capture(&wait_for_capture, "second save incarnation").await,
            1
        );

        let third = Arc::new(GatedBookshelfCapture::new(
            Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)),
            2,
            captured.clone(),
            resume_writer.clone(),
        ));
        let third_entity: Arc<dyn BlockEntity> = third.clone();
        world.add_block_entity(third_entity);
        third.bookshelf.set_book(1, ItemStack::new(1, &Item::BOOK));
        third.pause.store(true, Ordering::Release);
        resume_capture.send(()).unwrap();
        assert_eq!(
            wait_for_crossbeam_capture(&wait_for_capture, "third save incarnation").await,
            2
        );

        let fourth = Arc::new(GatedBookshelfCapture::new(
            Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)),
            3,
            captured,
            resume_writer,
        ));
        let fourth_entity: Arc<dyn BlockEntity> = fourth.clone();
        world.add_block_entity(fourth_entity);
        fourth.bookshelf.set_book(5, ItemStack::new(1, &Item::BOOK));
        resume_capture.send(()).unwrap();
        save.await.unwrap();

        world
            .block_entities
            .get_mut(&BOOKSHELF.chunk_position())
            .unwrap()
            .remove(&BOOKSHELF);
        let scenario = scenario.restart().await;
        let player = scenario
            .join(Uuid::new_v4(), "ReloadReplacementChurn")
            .await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&BOOKSHELF)
            .expect("final bookshelf incarnation missing after reload")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(5);
        assert_eq!(stack.item, &Item::BOOK);
        assert_eq!(stack.item_count, 1);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_replacements_retire_the_intermediate_container() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "RetireIntermediate").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (observed, wait_for_observed) = crossbeam::channel::bounded(2);
        let (resume_a, resume_writer_a) = crossbeam::channel::bounded(1);
        let (resume_b, resume_writer_b) = crossbeam::channel::bounded(1);
        let first = Arc::new(PredecessorGateChest {
            chest: Arc::new(ChestBlockEntity::new(CHEST)),
            calls: AtomicUsize::new(0),
            enabled: AtomicBool::new(false),
            observed,
            resume: [resume_writer_a, resume_writer_b],
        });
        let first_entity: Arc<dyn BlockEntity> = first.clone();
        world.add_block_entity(first_entity);
        first.calls.store(0, Ordering::Release);
        first.enabled.store(true, Ordering::Release);

        let middle = Arc::new(ChestBlockEntity::new(CHEST));
        middle.set_stack(0, ItemStack::new(4, &Item::EMERALD));
        let middle_inventory: Arc<dyn Inventory> = middle.clone();
        let last = Arc::new(ChestBlockEntity::new(CHEST));
        last.set_stack(0, ItemStack::new(6, &Item::DIAMOND));

        let first_world = world.clone();
        let first_replacement = middle.clone();
        let replace_middle = std::thread::spawn(move || {
            first_world.add_block_entity(first_replacement);
        });
        let first_observation = wait_for_crossbeam_barrier(
            &wait_for_observed,
            "first replacement observes predecessor",
        );

        let second_world = world.clone();
        let second_replacement = last.clone();
        let replace_last = std::thread::spawn(move || {
            second_world.add_block_entity(second_replacement);
        });
        let second_observation = wait_for_crossbeam_barrier(
            &wait_for_observed,
            "second replacement observes predecessor",
        );
        assert_ne!(first_observation, second_observation);

        [&resume_a, &resume_b][first_observation].send(()).unwrap();
        replace_middle.join().unwrap();
        [&resume_a, &resume_b][second_observation].send(()).unwrap();
        replace_last.join().unwrap();

        let late_transfer = middle_inventory.remove_stack_specific(0, 1);
        assert!(
            late_transfer.is_empty(),
            "intermediate chest remained a live transaction owner after replacement"
        );
        world.save().await;
        world
            .block_entities
            .get_mut(&CHEST.chunk_position())
            .unwrap()
            .remove(&CHEST);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadLastReplacement").await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&CHEST)
            .expect("last replacement missing after reload")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::DIAMOND);
        assert_eq!(stack.item_count, 6);
        scenario.stop().await;
    })
    .await;
}
