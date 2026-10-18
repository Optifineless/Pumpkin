use super::block_entity_support::*;
use super::*;
use crate::block::entities::{
    BlockEntity, chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering, mpsc};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_seal_rejects_a_bookshelf_replacement_and_keeps_captured_image() {
    run(async {
        let scenario = Scenario::new().await;
        let player_id = Uuid::new_v4();
        let player = scenario.join(player_id, "RetiringBookshelf").await;
        let world = player.player.world();
        world.set_block_state(
            &BOOKSHELF,
            Block::CHISELED_BOOKSHELF.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let original = Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF));
        original.set_book(0, ItemStack::new(1, &Item::BOOK));
        let paused = Arc::new(GatedBookshelfCapture {
            bookshelf: original,
            capture_id: 0,
            captured,
            resume: resume_writer,
            pause: AtomicBool::new(false),
        });
        let paused_entity: Arc<dyn BlockEntity> = paused.clone();
        world.add_block_entity(paused_entity.clone());
        paused.pause.store(true, Ordering::Release);

        let replacement = Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF));
        replacement.set_book(2, ItemStack::new(1, &Item::BOOK));
        let replacement_entity: Arc<dyn BlockEntity> = replacement.clone();
        let writer_world = world.clone();
        let writer = std::thread::spawn(move || {
            assert_eq!(
                wait_for_crossbeam_barrier(&wait_for_capture, "final bookshelf capture"),
                0
            );
            writer_world.add_block_entity(replacement_entity.clone());
            let replacement_was_admitted = writer_world
                .get_block_entity(&BOOKSHELF)
                .is_some_and(|current| Arc::ptr_eq(&current, &replacement_entity));
            resume_capture.send(()).unwrap();
            replacement_was_admitted
        });

        scenario.stop().await;
        assert!(
            !writer.join().unwrap(),
            "unload admitted a replacement after sealing block-entity mutations"
        );
        let scenario = scenario.restart().await;
        let player = scenario.join(player_id, "RetiringBookshelf").await;
        let world = player.player.world();
        let inventory = world
            .get_block_entity(&BOOKSHELF)
            .expect("reloaded chiseled bookshelf missing after stop")
            .get_inventory()
            .unwrap();
        assert_eq!(inventory.get_stack(0).item, &Item::BOOK);
        assert!(inventory.get_stack(2).is_empty());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_seal_rejects_a_replacement_before_the_final_chest_capture() {
    run(async {
        let scenario = Scenario::new().await;
        let player_id = Uuid::new_v4();
        scenario.join(player_id, "ChestRace").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        contents(&world);

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let original = world.get_block_entity(&CHEST).unwrap();
        let original_chest = original.clone().get_inventory().unwrap();
        let chest = original
            .as_any()
            .downcast_ref::<ChestBlockEntity>()
            .unwrap();
        let paused = Arc::new(PausedChestCapture {
            chest: Arc::new(ChestBlockEntity::from_nbt(
                &{
                    let mut nbt = NbtCompound::new();
                    chest.write_nbt(&mut nbt);
                    nbt
                },
                CHEST,
            )),
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        paused
            .clone()
            .get_inventory()
            .unwrap()
            .set_stack(0, original_chest.get_stack(0));
        let paused_entity: Arc<dyn BlockEntity> = paused.clone();
        world.add_block_entity(paused_entity.clone());
        paused.pause.store(true, Ordering::Release);

        let replacement_chest = Arc::new(ChestBlockEntity::new(CHEST));
        replacement_chest.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        let replacement_entity: Arc<dyn BlockEntity> = replacement_chest.clone();
        let replacement_world = world.clone();
        let writer = std::thread::spawn(move || {
            wait_for_std_barrier(&wait_for_capture, "stop final chest capture");
            replacement_world.add_block_entity(replacement_entity.clone());
            let replacement_was_admitted = replacement_world
                .get_block_entity(&CHEST)
                .is_some_and(|current| Arc::ptr_eq(&current, &replacement_entity));
            resume_capture.send(()).unwrap();
            replacement_was_admitted
        });

        scenario.stop().await;
        assert!(
            !writer.join().unwrap(),
            "server stop admitted a chest replacement after sealing mutations"
        );

        let scenario = scenario.restart().await;
        scenario.join(player_id, "ChestRace").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        let inventory = world
            .get_block_entity(&CHEST)
            .unwrap()
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::DIAMOND);
        assert_eq!(stack.item_count, 17);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunk_unload_retires_the_current_chest_before_item_command_mutation() {
    run(async {
        let scenario = Scenario::new().await;
        scenario.join(Uuid::new_v4(), "UnloadChestRace").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let original = Arc::new(PausedChestCapture {
            chest: Arc::new(ChestBlockEntity::new(CHEST)),
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        let original_entity: Arc<dyn BlockEntity> = original.clone();
        world.add_block_entity(original_entity.clone());
        original.pause.store(true, Ordering::Release);

        let unloading_world = world.clone();
        let unload = std::thread::spawn(move || {
            unloading_world.unload_block_entities(CHEST.chunk_position());
        });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "chunk unload final chest capture");
        })
        .await
        .unwrap();

        let replacement = Arc::new(ChestBlockEntity::new(CHEST));
        replacement.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        let replacement_entity: Arc<dyn BlockEntity> = replacement.clone();
        world.add_block_entity(replacement_entity.clone());
        assert!(
            world
                .get_block_entity(&CHEST)
                .is_some_and(|current| Arc::ptr_eq(&current, &original_entity))
        );

        assert_eq!(
            scenario.command(&format!(
                "item replace block {} {} {} container.0 with minecraft:diamond 1",
                CHEST.0.x, CHEST.0.y, CHEST.0.z
            )),
            1,
            "the item command did not reach the unloading chest"
        );
        assert!(original.get_inventory().unwrap().get_stack(0).is_empty());

        resume_capture.send(()).unwrap();
        unload.join().unwrap();
        assert!(
            !world
                .block_entities
                .get(&CHEST.chunk_position())
                .is_some_and(|entities| entities.contains_key(&CHEST)),
            "chunk unload retained a live block entity"
        );

        let scenario = scenario.restart().await;
        scenario.join(Uuid::new_v4(), "UnloadChestReload").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        let inventory = world
            .get_block_entity(&CHEST)
            .expect("unloaded chest missing after restart")
            .get_inventory()
            .unwrap();
        assert!(inventory.get_stack(0).is_empty());
        scenario.stop().await;
    })
    .await;
}
