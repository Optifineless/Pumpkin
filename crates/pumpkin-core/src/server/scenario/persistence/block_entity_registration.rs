use super::block_entity_round5_support::wait_for_crossbeam_capture;
use super::block_entity_support::*;
use super::*;
use crate::block::entities::BlockEntity;
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{Mutex, atomic::AtomicBool, atomic::Ordering, mpsc};

const LOOT_TABLE: &str = "minecraft:chests/simple_dungeon";
const LOOT_SEED: i64 = 0x191;

fn deferred_chest() -> Arc<ChestBlockEntity> {
    let mut chest = ChestBlockEntity::new(CHEST);
    *chest.loot_table.lock().unwrap() = Some(LOOT_TABLE.into());
    chest.loot_table_seed = LOOT_SEED;
    Arc::new(chest)
}

fn pending_image(world: &World) -> NbtCompound {
    let chunk = world
        .level
        .loaded_chunks
        .get(&CHEST.chunk_position())
        .unwrap()
        .clone();
    chunk
        .pending_block_entities
        .lock()
        .unwrap()
        .get(&CHEST)
        .cloned()
        .expect("registered chest has no persistent image")
}

fn detach_registration_image(world: &World) {
    // Reload the image under test, without a later shutdown capture repairing it.
    world
        .block_entities
        .get_mut(&CHEST.chunk_position())
        .unwrap()
        .remove(&CHEST);
}

fn assert_generated_contents(nbt: &NbtCompound) {
    assert_eq!(
        nbt.get_string("LootTable"),
        None,
        "delayed initial publication restored already-generated deferred loot"
    );
    let chest = ChestBlockEntity::from_nbt(nbt, CHEST);
    assert_eq!(chest.get_stack(0).item, &Item::EMERALD);
    assert_eq!(chest.get_stack(0).item_count, 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_registration_persists_complete_deferred_loot_before_a_later_save() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "InitialLootImage").await;
        let world = player.player.world();
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        world.add_block_entity(deferred_chest());

        let saved = pending_image(&world);
        assert_eq!(
            saved.get_string("LootTable"),
            Some(LOOT_TABLE),
            "initial registration persisted packet-only NBT instead of deferred loot"
        );
        assert_eq!(saved.get_long("LootTableSeed"), Some(LOOT_SEED));
        detach_registration_image(&world);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadInitialLoot").await;
        let loaded = player.player.world().get_block_entity(&CHEST).unwrap();
        assert_eq!(
            loaded.take_loot_table(),
            Some((LOOT_TABLE.into(), LOOT_SEED))
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_initial_registration_cannot_overwrite_a_retried_full_save() {
    run(async {
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "DelayedRegistration").await;
        let world = player.player.world();
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
        world.add_block_entity(original.clone());
        original.pause.store(true, Ordering::Release);
        let save_world = world.clone();
        let save = tokio::spawn(async move { save_world.save().await });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "original save capture");
        })
        .await
        .unwrap();

        let (registered, wait_for_registration) = crossbeam::channel::bounded(1);
        let (resume_registration, resume_publisher) = crossbeam::channel::bounded(1);
        *world.persistence_test_hooks.registration.lock().unwrap() = Some(Arc::new(move || {
            registered.send(()).unwrap();
            wait_for_crossbeam_barrier(&resume_publisher, "initial publication resume");
        }));
        let replacement = deferred_chest();
        let replacement_entity: Arc<dyn BlockEntity> = replacement.clone();
        let add_world = world.clone();
        let add_entity = replacement_entity.clone();
        let add = std::thread::spawn(move || add_world.add_block_entity(add_entity));
        wait_for_crossbeam_capture(&wait_for_registration, "replacement registered").await;
        *world.persistence_test_hooks.registration.lock().unwrap() = None;

        assert_eq!(
            replacement.take_loot_table(),
            Some((LOOT_TABLE.into(), LOOT_SEED))
        );
        replacement.set_stack(0, ItemStack::new(7, &Item::EMERALD));
        world.update_block_entity(&replacement_entity);
        resume_capture.send(()).unwrap();
        save.await.unwrap();
        assert_generated_contents(&pending_image(&world));
        resume_registration.send(()).unwrap();
        add.join().unwrap();
        assert_generated_contents(&pending_image(&world));

        detach_registration_image(&world);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadRegisteredSave").await;
        let world = player.player.world();
        let loaded = world.get_block_entity(&CHEST).unwrap();
        let mut nbt = NbtCompound::new();
        loaded.write_internal(&mut nbt);
        assert_generated_contents(&nbt);
        scenario.stop().await;
    })
    .await;
}
