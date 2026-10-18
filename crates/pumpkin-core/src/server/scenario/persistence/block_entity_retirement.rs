use super::block_entity_support::*;
use super::*;
use crate::block::entities::{
    BlockEntity, chest::ChestBlockEntity, chiseled_bookshelf::ChiseledBookshelfBlockEntity,
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, Mutex, mpsc};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retiring_bookshelf_replacement_churn_saves_the_admitted_current_entity() {
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
        let admitted = Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF));
        admitted.set_book(2, ItemStack::new(1, &Item::BOOK));
        let make_capture = |capture_id, bookshelf| {
            Arc::new(GatedBookshelfCapture {
                bookshelf,
                capture_id,
                captured: captured.clone(),
                resume: resume_writer.clone(),
            })
        };
        let second = make_capture(1, admitted);
        let second_entity: Arc<dyn BlockEntity> = second.clone();
        let second_identity = second_entity.clone();
        let third_entity: Arc<dyn BlockEntity> =
            make_capture(2, Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)));
        let first = make_capture(0, Arc::new(ChiseledBookshelfBlockEntity::new(BOOKSHELF)));
        world.add_block_entity(first);

        let writer_world = world.clone();
        let writer = std::thread::spawn(move || {
            assert_eq!(
                wait_for_crossbeam_barrier(&wait_for_capture, "first retiring capture"),
                0
            );
            writer_world.add_block_entity(second_entity);
            resume_capture.send(()).unwrap();

            assert_eq!(
                wait_for_crossbeam_barrier(&wait_for_capture, "replacement retiring capture"),
                1
            );
            writer_world.add_block_entity(third_entity);
            let replacement_was_admitted = writer_world
                .get_block_entity(&BOOKSHELF)
                .is_some_and(|current| !Arc::ptr_eq(&current, &second_identity));
            resume_capture.send(()).unwrap();
            assert!(
                !replacement_was_admitted,
                "sealed retiring capture admitted a later block entity replacement"
            );
        });

        scenario.stop().await;
        writer.join().unwrap();
        let scenario = scenario.restart().await;
        let player = scenario.join(player_id, "RetiringBookshelf").await;
        let world = player.player.world();
        let bookshelf = world
            .get_block_entity(&BOOKSHELF)
            .expect("reloaded chiseled bookshelf missing after stop");
        let inventory = bookshelf.get_inventory().unwrap();
        let stack = inventory.get_stack(2);
        assert_eq!(stack.item, &Item::BOOK);
        assert_eq!(stack.item_count, 1);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_does_not_publish_a_replaced_chests_stale_capture() {
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
        });
        paused
            .clone()
            .get_inventory()
            .unwrap()
            .set_stack(0, original_chest.get_stack(0));
        world.add_block_entity(paused);

        let (replacement_captured, wait_for_replacement) = mpsc::channel();
        let (resume_replacement, resume_replacement_writer) = mpsc::channel();
        let replacement_chest = Arc::new(ChestBlockEntity::new(CHEST));
        replacement_chest.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        let replacement_inventory: Arc<dyn Inventory> = replacement_chest.clone();
        let replacement = Arc::new(PausedChestCapture {
            chest: replacement_chest,
            captured: replacement_captured,
            resume: Mutex::new(Some(resume_replacement_writer)),
        });
        let replacement_world = world.clone();
        let (transfer_result, transferred) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            wait_for_std_barrier(&wait_for_capture, "stop original chest capture");
            replacement_world.add_block_entity(replacement);
            resume_capture.send(()).unwrap();
            wait_for_std_barrier(&wait_for_replacement, "stop replacement chest capture");

            // The retained handle must stop transferring items once this final image is captured.
            let moved = replacement_inventory.remove_stack_specific(0, 1);
            let remaining = replacement_inventory.get_stack(0);
            transfer_result.send((moved, remaining)).unwrap();
            resume_replacement.send(()).unwrap();
        });

        scenario.stop().await;
        writer.join().unwrap();
        let (moved, remaining) =
            wait_for_std_barrier(&transferred, "retired chest transfer result");
        assert!(
            moved.is_empty(),
            "retired chest allowed a late item transfer"
        );
        assert_eq!(remaining.item, &Item::EMERALD);
        assert_eq!(remaining.item_count, 9);

        let scenario = scenario.restart().await;
        scenario.join(player_id, "ChestRace").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        let inventory = world
            .get_block_entity(&CHEST)
            .unwrap()
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        scenario.stop().await;
        assert_eq!(stack.item, &Item::EMERALD);
        assert_eq!(stack.item_count, 9);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunk_unload_retires_replacement_before_item_command_can_mutate_it() {
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
        });
        world.add_block_entity(original);

        let unloading_world = world.clone();
        let unload = std::thread::spawn(move || {
            unloading_world.unload_block_entities(CHEST.chunk_position());
        });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "chunk unload original chest capture");
        })
        .await
        .unwrap();

        let (replacement_captured, wait_for_replacement) = mpsc::channel();
        let (resume_replacement, resume_replacement_writer) = mpsc::channel();
        let replacement_chest = Arc::new(ChestBlockEntity::new(CHEST));
        replacement_chest.set_stack(0, ItemStack::new(9, &Item::EMERALD));
        let replacement = Arc::new(PausedChestCapture {
            chest: replacement_chest,
            captured: replacement_captured,
            resume: Mutex::new(Some(resume_replacement_writer)),
        });
        world.add_block_entity(replacement);
        resume_capture.send(()).unwrap();
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(
                &wait_for_replacement,
                "chunk unload replacement chest capture",
            );
        })
        .await
        .unwrap();

        assert_eq!(
            scenario.command(&format!(
                "item replace block {} {} {} container.0 with minecraft:diamond 1",
                CHEST.0.x, CHEST.0.y, CHEST.0.z
            )),
            1,
            "the item command did not reach the captured replacement"
        );
        let chest = world
            .get_block_entity(&CHEST)
            .unwrap()
            .get_inventory()
            .unwrap();
        let after_command = chest.get_stack(0);
        assert_eq!(after_command.item, &Item::EMERALD);
        assert_eq!(after_command.item_count, 9);

        resume_replacement.send(()).unwrap();
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
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::EMERALD);
        assert_eq!(stack.item_count, 9);
        scenario.stop().await;
    })
    .await;
}
