use super::block_entity_support::*;
use super::*;
use crate::world::block_placer::WorldBlockPlacer;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::generation::structure::template::{
    BlockPlacer, PaletteEntry, StructureTemplate, TemplateBlock, place_template,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

async fn paused_chest_unload(
    scenario: &Scenario,
) -> (Arc<World>, mpsc::Sender<()>, std::thread::JoinHandle<bool>) {
    let world = scenario.world(&Dimension::OVERWORLD);
    world.set_block_state(
        &CHEST,
        Block::CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let chest = Arc::new(ChestBlockEntity::new(CHEST));
    chest.set_stack(0, ItemStack::new(7, &Item::EMERALD));
    let (captured, wait_for_capture) = mpsc::channel();
    let (resume_capture, resume_writer) = mpsc::channel();
    let paused = Arc::new(PausedChestCapture {
        chest,
        captured,
        resume: Mutex::new(Some(resume_writer)),
        pause: AtomicBool::new(false),
    });
    world.add_block_entity(paused.clone());
    paused.pause.store(true, Ordering::Release);
    let unload_world = world.clone();
    let unload =
        std::thread::spawn(move || unload_world.unload_block_entities(CHEST.chunk_position()));
    tokio::task::spawn_blocking(move || {
        wait_for_std_barrier(&wait_for_capture, "break/structure unload capture");
    })
    .await
    .unwrap();
    (world, resume_capture, unload)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_admission_rejects_block_break_before_drops() {
    run(async {
        let scenario = Scenario::new().await;
        scenario.join(Uuid::new_v4(), "RejectLateBreak").await;
        let (world, resume_capture, unload) = paused_chest_unload(&scenario).await;
        let entities_before = world.entities.load_full();

        let broken = world.break_block(&CHEST, None, BlockFlags::NOTIFY_ALL);
        let entities_after = world.entities.load_full();
        let terrain = world.get_block_state_id(&CHEST);
        resume_capture.send(()).unwrap();
        assert!(unload.join().unwrap());

        assert!(
            broken.is_none(),
            "late break ran after unload admission closed"
        );
        assert_eq!(
            entities_before.len(),
            entities_after.len(),
            "rejected break dropped loot"
        );
        assert_eq!(terrain, Block::CHEST.default_state.id);
        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadUnbrokenChest").await;
        let world = player.player.world();
        assert_eq!(
            world.get_block_state_id(&CHEST),
            Block::CHEST.default_state.id
        );
        let inventory = world
            .get_block_entity(&CHEST)
            .unwrap()
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
async fn rejected_structure_terrain_cannot_install_its_delayed_block_entity() {
    run(async {
        let scenario = Scenario::new().await;
        scenario.join(Uuid::new_v4(), "RejectLateTemplate").await;
        let (world, resume_capture, unload) = paused_chest_unload(&scenario).await;
        let position = BlockPos::new(9, 64, 8);
        let mut template = StructureTemplate::default();
        template.size = Vector3::new(1, 1, 1);
        template.palette = vec![PaletteEntry::new("minecraft:chest".into())];
        template.blocks = vec![TemplateBlock {
            pos: Vector3::new(0, 0, 0),
            state: 0,
            nbt: None,
        }];
        let mut placer = WorldBlockPlacer::new(&world);
        place_template(
            &mut placer,
            &template,
            position.0,
            (0, 0),
            pumpkin_data::Rotation::None,
            false,
            false,
            &[],
            None,
        );
        assert!(placer.changed_positions.is_empty());
        resume_capture.send(()).unwrap();
        assert!(unload.join().unwrap());
        let storage = world
            .level
            .get_entity_chunk(CHEST.chunk_position())
            .await
            .unwrap();
        assert!(world.make_chunk_entities_live(&storage, None));
        // A legitimate reactivation cannot authorize NBT from the rejected earlier terrain write.
        placer.finalize();
        assert!(
            world.get_block_entity(&position).is_none(),
            "rejected template installed a block entity after its terrain write failed"
        );
        assert_eq!(
            world.get_block_state_id(&position),
            Block::AIR.default_state.id
        );
        drop(placer);

        // Feature deltas may contain NBT for unchanged terrain; those records still need admission.
        world.set_block_state(
            &position,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let chest = ChestBlockEntity::new(position);
        chest.set_stack(0, ItemStack::new(3, &Item::DIAMOND));
        let mut nbt = NbtCompound::new();
        crate::block::entities::BlockEntity::write_internal(&chest, &mut nbt);
        let mut unchanged = WorldBlockPlacer::new(&world);
        unchanged.add_block_entity(nbt);
        unchanged.finalize();
        drop(unchanged);
        let scenario = scenario.restart().await;
        let player = scenario
            .join(Uuid::new_v4(), "ReloadUnchangedTemplate")
            .await;
        let inventory = player
            .player
            .world()
            .get_block_entity(&position)
            .unwrap()
            .get_inventory()
            .unwrap();
        let stack = inventory.get_stack(0);
        assert_eq!(stack.item, &Item::DIAMOND);
        assert_eq!(stack.item_count, 3);
        scenario.stop().await;
    })
    .await;
}
