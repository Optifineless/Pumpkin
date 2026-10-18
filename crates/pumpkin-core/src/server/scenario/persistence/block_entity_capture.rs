use super::block_entity_support::*;
use super::*;
use crate::block::entities::{
    chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering, mpsc};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_all_replacement_keeps_block_entity_custom_data_after_reload() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "SaveCustomRace").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        world.set_block_entity_custom_data(
            &CHEST,
            "test:plugin",
            "marker",
            pumpkin_nbt::tag::NbtTag::String("survives".into()),
        );

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let old = Arc::new(ChestBlockEntity::new(CHEST));
        let paused = Arc::new(PausedChestCapture {
            chest: old,
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(paused.clone());
        paused.pause.store(true, Ordering::Release);

        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "save-all chest capture");
        })
        .await
        .unwrap();

        let replacement = Arc::new(ChestBlockEntity::new(CHEST));
        replacement.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        world.add_block_entity(replacement);
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
            .expect("World::save left no chest save image");
        let mut namespace = NbtCompound::new();
        namespace.put_string("marker", "survives".into());
        let mut expected_custom = NbtCompound::new();
        expected_custom.put_compound("test:plugin", namespace);
        assert_eq!(
            saved.get_compound("PumpkinCustomData").cloned(),
            Some(expected_custom),
            "replacement save image lost block-entity custom data"
        );

        // Preserve the World::save image through shutdown so reload tests that exact image.
        world
            .block_entities
            .get_mut(&CHEST.chunk_position())
            .unwrap()
            .remove(&CHEST);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "LoadCustomRace").await;
        let world = player.player.world();
        assert!(world.get_block_entity(&CHEST).is_some());
        assert_eq!(
            world.get_block_entity_custom_data(&CHEST, "test:plugin", "marker"),
            Some(pumpkin_nbt::tag::NbtTag::String("survives".into()))
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_same_arc_saves_keep_newer_chest_contents_after_reload() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "OverlappingSave").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let chest = Arc::new(ChestBlockEntity::new(CHEST));
        chest.set_stack(0, ItemStack::new(3, &Item::DIAMOND));
        let paused = Arc::new(PausedChestCapture {
            chest: chest.clone(),
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(paused.clone());
        paused.pause.store(true, Ordering::Release);

        let older_world = world.clone();
        let older_save = tokio::spawn(async move { older_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "overlapping chest capture");
        })
        .await
        .unwrap();

        chest.set_stack(0, ItemStack::new(7, &Item::EMERALD));
        world.clone().save().await;
        resume_capture.send(()).unwrap();
        older_save.await.unwrap();

        // Keep shutdown from taking a later snapshot and masking the World::save ordering.
        world
            .block_entities
            .get_mut(&CHEST.chunk_position())
            .unwrap()
            .remove(&CHEST);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadOverlappingSave").await;
        let world = player.player.world();
        let inventory = world
            .get_block_entity(&CHEST)
            .expect("reloaded chest missing after overlapping saves")
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::EMERALD);
        assert_eq!(stack.item_count, 7);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_capture_retries_default_none_block_entity_state() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "SaveBookshelfRace").await;
        let world = player.player.world();
        world.set_block_state(
            &BOOKSHELF,
            Block::CHISELED_BOOKSHELF.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let old = Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF));
        old.set_book(0, ItemStack::new(1, &Item::BOOK));
        let paused = Arc::new(PausedBookshelfCapture {
            bookshelf: old,
            captured,
            resume: resume_writer,
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(paused.clone());
        paused.pause.store(true, Ordering::Release);

        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_crossbeam_barrier(&wait_for_capture, "bookshelf replacement capture");
        })
        .await
        .unwrap();

        let replacement = Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF));
        replacement.set_book(4, ItemStack::new(1, &Item::BOOK));
        world.add_block_entity(replacement);
        resume_capture.send(()).unwrap();
        save.await.unwrap();

        let chunk = world
            .level
            .loaded_chunks
            .get(&BOOKSHELF.chunk_position())
            .unwrap()
            .clone();
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&BOOKSHELF)
            .cloned()
            .expect("default-None block entity had no fresh save image");
        assert_eq!(
            saved
                .get_list("Items")
                .map(<[pumpkin_nbt::tag::NbtTag]>::len),
            Some(1),
            "World::save did not capture the replacement bookshelf inventory"
        );

        // Preserve the World::save image through shutdown so reload tests that exact image.
        world
            .block_entities
            .get_mut(&BOOKSHELF.chunk_position())
            .unwrap()
            .remove(&BOOKSHELF);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "LoadBookshelfRace").await;
        let world = player.player.world();
        let bookshelf = world
            .get_block_entity(&BOOKSHELF)
            .expect("reloaded bookshelf missing after save-all race");
        let inventory = bookshelf.get_inventory().unwrap();
        let saved_book = inventory.get_stack(4);
        assert_eq!(saved_book.item, &Item::BOOK);
        assert_eq!(saved_book.item_count, 1);
        scenario.stop().await;
    })
    .await;
}
