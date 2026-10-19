use super::block_entity_support::*;
use super::*;
use crate::block::entities::BlockEntity;
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const SECOND_CHEST: BlockPos = BlockPos::new(10, 64, 8);

struct ChestCaptureGate {
    entity: Arc<PausedChestCapture>,
    captured: mpsc::Receiver<()>,
    resume: mpsc::Sender<()>,
}

impl ChestCaptureGate {
    fn new(world: &Arc<World>, pos: BlockPos) -> Self {
        world.set_block_state(&pos, Block::CHEST.default_state.id, BlockFlags::FORCE_STATE);
        let (captured, receiver) = mpsc::channel();
        let (resume, writer) = mpsc::channel();
        let chest = Arc::new(ChestBlockEntity::new(pos));
        chest.set_stack(0, ItemStack::new(3, &Item::DIAMOND));
        let entity = Arc::new(PausedChestCapture {
            chest,
            captured,
            resume: Mutex::new(Some(writer)),
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(entity.clone());
        Self {
            entity,
            captured: receiver,
            resume,
        }
    }
}

fn gates_in_save_order(world: &Arc<World>) -> (ChestCaptureGate, ChestCaptureGate) {
    let left = ChestCaptureGate::new(world, CHEST);
    let right = ChestCaptureGate::new(world, SECOND_CHEST);
    // Both passes collect the unchanged registry in the same order. Blocking its second
    // capture keeps the first final image published while its original Arc is registered.
    let first_pos = world
        .block_entities
        .get(&CHEST.chunk_position())
        .unwrap()
        .values()
        .find(|entity| [CHEST, SECOND_CHEST].contains(&entity.get_position()))
        .unwrap()
        .get_position();
    if left.entity.get_position() == first_pos {
        (left, right)
    } else {
        (right, left)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_save_cannot_overwrite_same_arc_final_unload_image() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "SaveUnloadRace").await;
        let world = player.player.world();
        let (first, second) = gates_in_save_order(&world);
        let first_pos = first.entity.get_position();
        first.entity.pause.store(true, Ordering::Release);
        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        let first_capture = first.captured;
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&first_capture, "ordinary save old chest image");
        })
        .await
        .unwrap();

        first
            .entity
            .chest
            .set_stack(0, ItemStack::new(7, &Item::EMERALD));
        second.entity.pause.store(true, Ordering::Release);
        let unload_world = world.clone();
        let unload =
            std::thread::spawn(move || unload_world.unload_block_entities(CHEST.chunk_position()));
        let second_capture = second.captured;
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&second_capture, "unload after first final publication");
        })
        .await
        .unwrap();
        let expected: Arc<dyn BlockEntity> = first.entity.clone();
        assert!(Arc::ptr_eq(
            &world.get_block_entity(&first_pos).unwrap(),
            &expected
        ));
        first.resume.send(()).unwrap();
        save.await.unwrap();

        let saved = world
            .level
            .loaded_chunks
            .get(&first_pos.chunk_position())
            .unwrap()
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&first_pos)
            .cloned()
            .unwrap();
        let saved_chest = ChestBlockEntity::from_nbt(&saved, first_pos);
        second.resume.send(()).unwrap();
        assert!(unload.join().unwrap());
        assert!(
            !world
                .block_entities
                .contains_key(&first_pos.chunk_position())
        );
        assert_eq!(
            saved_chest.get_stack(0).item,
            &Item::EMERALD,
            "ordinary save replaced the same-Arc final unload image"
        );
        assert_eq!(saved_chest.get_stack(0).item_count, 7);

        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadSaveUnloadRace").await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&first_pos)
            .expect("same-Arc unload chest missing after reload")
            .get_inventory()
            .unwrap();
        assert_eq!(inventory.get_stack(0).item, &Item::EMERALD);
        assert_eq!(inventory.get_stack(0).item_count, 7);
        scenario.stop().await;
    })
    .await;
}
