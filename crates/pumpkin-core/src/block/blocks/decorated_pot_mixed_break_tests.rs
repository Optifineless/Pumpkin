use super::*;
use crate::{
    plugin::block::block_drop_item::BlockDropItemEvent,
    world::explosion::{BlockInteraction, Explosion},
};

struct ArrowInsideDrop {
    arrow: Arc<dyn EntityBase>,
    calls: Arc<AtomicUsize>,
    replacement: bool,
}

impl EventHandler<BlockDropItemEvent> for ArrowInsideDrop {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        if self.calls.fetch_add(1, Relaxed) == 0 {
            self.arrow.on_hit(impact(event.block_pos));
            if self.replacement {
                event.world.set_block_state(
                    &event.block_pos,
                    Block::DECORATED_POT.default_state.id,
                    BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
                let replacement = event.world.get_block_entity(&event.block_pos).unwrap();
                replacement
                    .as_any()
                    .downcast_ref::<crate::block::entities::decorated_pot::DecoratedPotBlockEntity>(
                    )
                    .unwrap()
                    .set_stack(0, ItemStack::new(1, &Item::EMERALD));
            }
        }
        Box::pin(async {})
    }
}

fn assert_conserved(fixture: &Fixture) {
    let mut faces: Vec<(u16, String)> = Vec::new();
    for entity in fixture.world.entities.load().iter() {
        let Some(item) = entity.get_item_entity() else {
            continue;
        };
        let stack = item.get_item_stack().lock().unwrap();
        if stack.item == &Item::DECORATED_POT {
            assert!(
                stack
                    .get_data_component::<pumpkin_data::data_component_impl::ContainerImpl>()
                    .is_none_or(|container| container.items.is_empty()),
                "stored contents are scattered once, never retained in the intact-pot drop"
            );
            let decorations = stack.get_data_component::<PotDecorationsImpl>().unwrap();
            for face in decorations.decorations.iter().flatten() {
                let stack = face.to_item_stack();
                faces.push((stack.item.id, stack.get_hover_name()));
            }
        } else if stack.item != &Item::DIAMOND && stack.item != &Item::EMERALD {
            for _ in 0..stack.item_count {
                faces.push((stack.item.id, stack.get_hover_name()));
            }
        }
    }
    faces.sort();
    let mut expected: Vec<_> = FACES
        .into_iter()
        .map(|(item, name)| (item.id, name.to_owned()))
        .collect();
    expected.sort();
    assert_eq!(faces, expected, "one complete original face set survives");
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 1);
    assert_eq!(item_count(&fixture.world, &Item::EMERALD), 0);
}

#[derive(Clone, Copy, Debug)]
enum Destroyer {
    FinishedDigging,
    Plugin,
    Piston,
    Explosion,
}

fn destroy(fixture: &Fixture, position: BlockPos, destroyer: Destroyer) {
    match destroyer {
        Destroyer::FinishedDigging => {
            fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
            fixture.dig(position);
        }
        Destroyer::Plugin => {
            fixture
                .world
                .break_block(&position, None, BlockFlags::NOTIFY_ALL);
        }
        Destroyer::Piston => {
            let piston = position.offset(Vector3::new(-1, 0, 0));
            let state = Block::PISTON
                .from_properties(&[("facing", "east"), ("extended", "false")])
                .to_state_id(&Block::PISTON);
            fixture
                .world
                .set_block_state(&piston, state, BlockFlags::FORCE_STATE);
            fixture
                .world
                .active_chunks
                .write()
                .unwrap()
                .insert(position.chunk_position());
            fixture.world.set_block_state(
                &piston.offset(Vector3::new(-1, 0, 0)),
                Block::REDSTONE_BLOCK.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            fixture.world.flush_synced_block_events();
            assert_eq!(fixture.world.get_block(&position), &Block::MOVING_PISTON);
        }
        Destroyer::Explosion => {
            // Centering the blast in the pot ensures every ray begins in this block.
            Explosion::new(0.5, position.to_centered_f64(), BlockInteraction::Destroy)
                .explode(&fixture.world);
        }
    }
}

fn destroy_on_worker(fixture: Arc<Fixture>, position: BlockPos, destroyer: Destroyer) {
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait_for_finish) = crossbeam::channel::bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        destroy(&fixture, position, destroyer);
        finished.send(()).unwrap();
    }));
    wait_for_finish
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
}

async fn mixed_destroyer_claims_before_arrow(destroyer: Destroyer) {
    let fixture = Arc::new(Fixture::new());
    let position = crafted_content_pot(&fixture);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockDropItemEvent, _>(
            Arc::new(ArrowInsideDrop {
                arrow,
                calls: calls.clone(),
                replacement: false,
            }),
            EventPriority::Normal,
            true,
        );
    destroy_on_worker(fixture.clone(), position, destroyer);
    assert_conserved(&fixture);
    assert_eq!(
        calls.load(Relaxed),
        1,
        "{destroyer:?} owns the original loot"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_mixed_finished_digging_first_arrow_callback() {
    mixed_destroyer_claims_before_arrow(Destroyer::FinishedDigging).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_mixed_plugin_break_first_arrow_callback() {
    mixed_destroyer_claims_before_arrow(Destroyer::Plugin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_mixed_piston_first_arrow_callback() {
    mixed_destroyer_claims_before_arrow(Destroyer::Piston).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_mixed_explosion_first_arrow_callback() {
    mixed_destroyer_claims_before_arrow(Destroyer::Explosion).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_mixed_drop_callback_replacement_survives() {
    for destroyer in [
        Destroyer::FinishedDigging,
        Destroyer::Plugin,
        Destroyer::Explosion,
    ] {
        let fixture = Arc::new(Fixture::new());
        let position = crafted_content_pot(&fixture);
        let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
        let calls = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<BlockDropItemEvent, _>(
                Arc::new(ArrowInsideDrop {
                    arrow,
                    calls: calls.clone(),
                    replacement: true,
                }),
                EventPriority::Normal,
                true,
            );
        destroy_on_worker(fixture.clone(), position, destroyer);
        assert_conserved(&fixture);
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);
        let replacement = fixture.world.get_block_entity(&position).unwrap();
        assert_eq!(
            replacement
                .as_any()
                .downcast_ref::<crate::block::entities::decorated_pot::DecoratedPotBlockEntity>()
                .unwrap()
                .get_item()
                .unwrap()
                .item,
            &Item::EMERALD,
        );
        fixture.shutdown().await;
    }
}
