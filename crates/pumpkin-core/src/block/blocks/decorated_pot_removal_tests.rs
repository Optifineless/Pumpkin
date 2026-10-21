use super::*;
use crate::{
    block::{
        entities::{BlockEntity, decorated_pot::DecoratedPotBlockEntity},
        fluid::{FluidBehaviour, water::FlowingWater},
    },
    command::context::command_source::CommandSource,
};
use pumpkin_data::fluid::Fluid;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_move_skip_callbacks_preserves_one_contents_image() {
    let fixture = Fixture::new();
    let source = crafted_content_pot(&fixture);
    let mut saved = NbtCompound::new();
    fixture
        .world
        .get_block_entity(&source)
        .unwrap()
        .write_internal(&mut saved);
    assert!(
        fixture
            .world
            .break_block(
                &source,
                None,
                BlockFlags::NOTIFY_ALL
                    | BlockFlags::SKIP_DROPS
                    | BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK,
            )
            .is_some()
    );
    assert_eq!(fixture.world.get_block(&source), &Block::AIR);
    assert!(fixture.world.get_block_entity(&source).is_none());
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 0);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    let destination = source.offset(Vector3::new(2, 0, 0));
    fixture.set_block(destination.down(), &Block::STONE);
    fixture.set_block(destination, &Block::DECORATED_POT);
    fixture
        .world
        .add_block_entity(Arc::new(DecoratedPotBlockEntity::from_nbt(
            &saved,
            destination,
        )));
    let restored = fixture.world.get_block_entity(&destination).unwrap();
    let diamond = restored
        .as_any()
        .downcast_ref::<DecoratedPotBlockEntity>()
        .unwrap()
        .get_item()
        .unwrap();
    assert_eq!((diamond.item, diamond.item_count), (&Item::DIAMOND, 1));
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 0);
    assert!(
        fixture
            .world
            .break_block(&destination, None, BlockFlags::NOTIFY_ALL)
            .is_some()
    );
    assert!(fixture.world.get_block_entity(&destination).is_none());
    mixed_break::assert_conserved(&fixture);
    fixture.shutdown().await;
}

fn replacement_item() -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::DECORATED_POT);
    stack.set_data_component(decorated([
        &Item::HEART_POTTERY_SHERD,
        &Item::SHEAF_POTTERY_SHERD,
        &Item::EXPLORER_POTTERY_SHERD,
        &Item::BLADE_POTTERY_SHERD,
    ]));
    stack
}

struct ArrowWinsBreak {
    position: BlockPos,
    world: Arc<World>,
    arrow: Arc<dyn EntityBase>,
    calls: Arc<AtomicUsize>,
}

impl EventHandler<BlockBreakEvent> for ArrowWinsBreak {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block_position == self.position && self.calls.fetch_add(1, Relaxed) == 0 {
            self.arrow.on_hit(impact(self.position));
            self.world.set_block_state(
                &self.position,
                Block::DECORATED_POT.default_state.id,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
            let replacement = self.world.get_block_entity(&self.position).unwrap();
            replacement.apply_item_components(&replacement_item());
            replacement
                .as_any()
                .downcast_ref::<DecoratedPotBlockEntity>()
                .unwrap()
                .set_stack(0, ItemStack::new(1, &Item::EMERALD));
        }
        Box::pin(async {})
    }
}

#[derive(Clone, Copy, Debug)]
enum Writer {
    Piston,
    Setblock,
    Fill,
    FluidTick,
}

fn write_after_break(fixture: &Fixture, position: BlockPos, writer: Writer) {
    match writer {
        Writer::Piston => mixed_break::power_piston(fixture, position),
        Writer::Setblock | Writer::Fill => {
            let mut source = CommandSource::dummy();
            source.server = Some(fixture.server.clone());
            source.world = Some(fixture.world.clone());
            source.silent = true;
            let coordinates = format!("{} {} {}", position.0.x, position.0.y, position.0.z);
            let command = match writer {
                Writer::Setblock => format!("setblock {coordinates} stone destroy"),
                _ => format!("fill {coordinates} {coordinates} stone destroy"),
            };
            assert!(
                fixture
                    .server
                    .command_dispatcher
                    .load()
                    .execute_input(&command, &source)
                    .is_err(),
                "{writer:?} must reject placement after losing destruction"
            );
        }
        Writer::FluidTick => {
            let water = position.up();
            fixture.set_block(water, &Block::WATER);
            FlowingWater.on_scheduled_tick(&fixture.world, &Fluid::FLOWING_WATER, &water);
        }
    }
}

async fn rejected_break_preserves_replacement(writer: Writer) {
    let fixture = Arc::new(Fixture::new());
    let position = crafted_content_pot(&fixture);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(
            Arc::new(ArrowWinsBreak {
                position,
                world: fixture.world.clone(),
                arrow,
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let worker_fixture = fixture.clone();
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait) = crossbeam::channel::bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        write_after_break(&worker_fixture, position, writer);
        finished.send(()).unwrap();
    }));
    wait.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(calls.load(Relaxed), 2, "outer and winning arrow admission");
    mixed_break::assert_conserved(&fixture);
    assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);
    assert!(
        !DecoratedPotLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
            .cracked
    );
    let replacement = fixture.world.get_block_entity(&position).unwrap();
    let mut image = ItemStack::new(1, &Item::DECORATED_POT);
    replacement.collect_item_components(&mut image);
    assert_eq!(
        image.get_data_component::<PotDecorationsImpl>(),
        replacement_item().get_data_component::<PotDecorationsImpl>()
    );
    let emerald = replacement
        .as_any()
        .downcast_ref::<DecoratedPotBlockEntity>()
        .unwrap()
        .get_item()
        .unwrap();
    assert_eq!((emerald.item, emerald.item_count), (&Item::EMERALD, 1));
    if matches!(writer, Writer::Piston) {
        let piston = position.offset(Vector3::new(-1, 0, 0));
        assert!(
            !pumpkin_data::block_properties::StickyPistonLikeProperties::from_state_id(
                fixture.world.get_block_state_id(&piston)
            )
            .extended
        );
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_rejected_piston_break_preserves_arrow_callback_replacement() {
    rejected_break_preserves_replacement(Writer::Piston).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_rejected_command_breaks_preserve_arrow_callback_replacement() {
    for writer in [Writer::Setblock, Writer::Fill] {
        rejected_break_preserves_replacement(writer).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_rejected_fluid_break_preserves_arrow_callback_replacement() {
    rejected_break_preserves_replacement(Writer::FluidTick).await;
}
