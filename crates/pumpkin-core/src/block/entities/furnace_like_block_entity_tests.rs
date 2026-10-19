#![expect(
    clippy::unwrap_used,
    reason = "Focused world fixtures require successful construction"
)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use pumpkin_data::{
    Block,
    biome::Biome,
    data_component_impl::{
        CookingFuelImpl, CustomNameImpl, FloatProvider, IntProvider, UseRemainderImpl,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{math::position::BlockPos, math::vector2::Vector2, text::TextComponent};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::entities::{
        BlockEntity, blasting_furnace::BlastingFurnaceBlockEntity, block_entity_from_nbt,
        furnace::FurnaceBlockEntity, furnace_like_block_entity::CookingBlockEntityBase,
        smoker::SmokerBlockEntity,
    },
    entity::death_test_world::DeathTestWorld,
    plugin::{BoxFuture, EventHandler, EventPriority, api::events::inventory::*},
    server::Server,
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

async fn run(action: impl FnOnce(&DeathTestWorld)) {
    let fixture = DeathTestWorld::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&fixture)));
    fixture.server.shutdown().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn prepare_world(world: &Arc<World>) {
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
}

fn furnace(world: &Arc<World>, position: BlockPos) -> Arc<FurnaceBlockEntity> {
    world.set_block_state(
        &position,
        Block::FURNACE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let entity = Arc::new(FurnaceBlockEntity::new(position));
    world.add_block_entity(entity.clone());
    entity
}

fn smoker(world: &Arc<World>, position: BlockPos) -> Arc<SmokerBlockEntity> {
    world.set_block_state(
        &position,
        Block::SMOKER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let entity = Arc::new(SmokerBlockEntity::new(position));
    world.add_block_entity(entity.clone());
    entity
}

fn blasting_furnace(world: &Arc<World>, position: BlockPos) -> Arc<BlastingFurnaceBlockEntity> {
    world.set_block_state(
        &position,
        Block::BLAST_FURNACE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let entity = Arc::new(BlastingFurnaceBlockEntity::new(position));
    world.add_block_entity(entity.clone());
    entity
}

fn tick(fixture: &DeathTestWorld, count: usize) {
    let world = fixture.world();
    for _ in 0..count {
        world.tick(&fixture.server);
    }
}

fn custom_fuel(burn_time: i32, speed_multiplier: f32) -> ItemStack {
    let mut fuel = ItemStack::new(1, &Item::STICK);
    fuel.set_data_component(CookingFuelImpl {
        burn_time: IntProvider::Inline(burn_time),
        speed_multiplier: FloatProvider::Inline(speed_multiplier),
    });
    fuel
}

#[tokio::test]
async fn b1019_g01_lava_remainder_and_fuel_controls() {
    run(|fixture| {
        let world = fixture.world();
        prepare_world(&world);

        let lava = furnace(&world, BlockPos::new(8, 64, 8));
        lava.set_stack(0, ItemStack::new(1, &Item::RAW_IRON));
        lava.set_stack(1, ItemStack::new(1, &Item::LAVA_BUCKET));

        let coal = furnace(&world, BlockPos::new(9, 64, 8));
        coal.set_stack(0, ItemStack::new(1, &Item::RAW_IRON));
        coal.set_stack(1, ItemStack::new(1, &Item::COAL));

        let invalid = furnace(&world, BlockPos::new(10, 64, 8));
        invalid.set_stack(0, ItemStack::new(1, &Item::DIRT));
        invalid.set_stack(1, ItemStack::new(1, &Item::COAL));

        let full = furnace(&world, BlockPos::new(11, 64, 8));
        full.set_stack(0, ItemStack::new(1, &Item::RAW_IRON));
        full.set_stack(1, ItemStack::new(1, &Item::COAL));
        full.set_stack(2, ItemStack::new(64, &Item::IRON_INGOT));

        tick(fixture, 1);

        assert!(
            coal.get_stack(1).is_empty(),
            "coal should leave no remainder"
        );
        assert!(coal.is_burning());
        assert_eq!(invalid.get_stack(1).item, &Item::COAL);
        assert!(!invalid.is_burning());
        assert_eq!(invalid.get_cooking_time_spent(), 0);
        assert_eq!(full.get_stack(1).item, &Item::COAL);
        assert!(!full.is_burning(), "a full output slot must block fuel use");
        assert_eq!(full.get_stack(2).item_count, 64);

        assert_eq!(lava.get_stack(1).item, &Item::BUCKET);
        assert_eq!(lava.get_stack(1).item_count, 1);
        assert!(lava.is_burning(), "lava bucket should start a valid cook");
        assert_eq!(lava.get_cooking_time_spent(), 1);
    })
    .await;
}

#[tokio::test]
async fn b1019_g01_lava_template_overflow_keeps_components_once() {
    run(|fixture| {
        let world = fixture.world();
        prepare_world(&world);

        let entity = furnace(&world, BlockPos::new(8, 64, 8));
        entity.set_stack(0, ItemStack::new(1, &Item::RAW_IRON));
        let mut remainder = ItemStack::new(2, &Item::DIAMOND);
        remainder.set_data_component(CustomNameImpl {
            name: TextComponent::text("fuel remainder template"),
        });
        let mut fuel = ItemStack::new(2, &Item::LAVA_BUCKET);
        fuel.set_data_component(CookingFuelImpl {
            burn_time: IntProvider::Inline(20),
            speed_multiplier: FloatProvider::Inline(1.0),
        });
        fuel.set_data_component(UseRemainderImpl {
            remainder: None,
            template: Some(Box::new(remainder)),
        });
        entity.set_stack(1, fuel);

        tick(fixture, 1);

        let remaining_fuel = entity.get_stack(1);
        assert_eq!(remaining_fuel.item, &Item::LAVA_BUCKET);
        assert_eq!(remaining_fuel.item_count, 1);
        let dropped: Vec<_> = world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .filter_map(|entity| {
                let stack = entity.get_item_stack().lock().unwrap().clone();
                (stack.item == &Item::DIAMOND).then_some(stack)
            })
            .collect();
        assert_eq!(dropped.len(), 1, "the template remainder must drop once");
        assert_eq!(dropped[0].item_count, 2);
        assert_eq!(
            dropped[0]
                .get_data_component::<CustomNameImpl>()
                .unwrap()
                .name
                .clone()
                .get_text(),
            "fuel remainder template"
        );
    })
    .await;
}

#[tokio::test]
async fn b1019_g01_appliance_speed_boundaries_and_recipe_kinds() {
    run(|fixture| {
        let world = fixture.world();
        prepare_world(&world);

        let furnace = furnace(&world, BlockPos::new(8, 64, 8));
        furnace.set_stack(0, ItemStack::new(10, &Item::RAW_IRON));
        furnace.set_stack(1, ItemStack::new(1, &Item::COAL));

        let smoker = smoker(&world, BlockPos::new(9, 64, 8));
        smoker.set_stack(0, ItemStack::new(10, &Item::POTATO));
        smoker.set_stack(1, ItemStack::new(1, &Item::COAL));

        let blast = blasting_furnace(&world, BlockPos::new(10, 64, 8));
        blast.set_stack(0, ItemStack::new(10, &Item::IRON_ORE));
        blast.set_stack(1, ItemStack::new(1, &Item::COAL));

        tick(fixture, 100);
        assert_eq!(furnace.get_stack(2).item_count, 0);
        assert_eq!(smoker.get_stack(2).item_count, 1);
        assert_eq!(smoker.get_stack(2).item, &Item::BAKED_POTATO);
        assert_eq!(blast.get_stack(2).item_count, 1);
        assert_eq!(blast.get_stack(2).item, &Item::IRON_INGOT);

        tick(fixture, 100);
        assert_eq!(furnace.get_stack(2).item_count, 1);
        assert_eq!(smoker.get_stack(2).item_count, 2);
        assert_eq!(blast.get_stack(2).item_count, 2);

        tick(fixture, 600);
        assert_eq!(furnace.get_stack(2).item_count, 4);
        assert_eq!(smoker.get_stack(2).item_count, 8);
        assert_eq!(blast.get_stack(2).item_count, 8);
        assert!(furnace.get_stack(2).item_count < 10);
        assert!(smoker.get_stack(2).item_count < 10);
        assert!(blast.get_stack(2).item_count < 10);
    })
    .await;
}

#[tokio::test]
async fn b1019_g01_speed_change_survives_factory_reload_and_legacy_default() {
    run(|fixture| {
        let world = fixture.world();
        prepare_world(&world);

        let smoker = smoker(&world, BlockPos::new(8, 64, 8));
        smoker.set_stack(0, ItemStack::new(2, &Item::POTATO));
        smoker.set_stack(1, custom_fuel(200, 1.0));

        tick(fixture, 100);
        assert_eq!(smoker.get_cooking_total_time(), 200);
        assert_eq!(smoker.get_cooking_time_spent(), 100);
        assert_eq!(smoker.get_stack(2).item_count, 0);

        smoker.set_stack(1, ItemStack::new(1, &Item::COAL));
        tick(fixture, 1);
        assert_eq!(smoker.get_speed_multiplier(), 2.0);
        assert_eq!(smoker.get_cooking_total_time(), 100);
        assert_eq!(smoker.get_cooking_time_spent(), 51);

        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
        smoker.write_internal(&mut nbt);
        let restored = block_entity_from_nbt(&nbt).expect("factory should restore the smoker");
        let restored_smoker = restored
            .as_any()
            .downcast_ref::<SmokerBlockEntity>()
            .expect("factory should preserve the concrete smoker type");
        assert_eq!(restored_smoker.get_speed_multiplier(), 2.0);
        assert_eq!(restored_smoker.get_cooking_total_time(), 100);
        assert_eq!(restored_smoker.get_cooking_time_spent(), 51);
        world.add_block_entity(restored.clone());

        tick(fixture, 48);
        assert_eq!(restored_smoker.get_cooking_time_spent(), 99);
        assert_eq!(restored_smoker.get_stack(2).item_count, 0);
        tick(fixture, 1);
        assert_eq!(restored_smoker.get_stack(2).item, &Item::BAKED_POTATO);
        assert_eq!(restored_smoker.get_stack(2).item_count, 1);
        tick(fixture, 99);
        assert_eq!(restored_smoker.get_stack(2).item_count, 1);
        tick(fixture, 1);
        assert_eq!(restored_smoker.get_stack(2).item_count, 2);

        let mut old_save = nbt;
        old_save.child_tags.remove("cooking_speed_multiplier");
        let legacy = block_entity_from_nbt(&old_save).expect("factory should read old saves");
        let legacy_smoker = legacy
            .as_any()
            .downcast_ref::<SmokerBlockEntity>()
            .expect("old save should retain its entity type");
        assert_eq!(legacy_smoker.get_speed_multiplier(), 1.0);
        assert_eq!(legacy_smoker.get_cooking_total_time(), 100);
        assert_eq!(legacy_smoker.get_cooking_time_spent(), 51);
    })
    .await;
}

struct FuelSwap {
    furnace: Arc<FurnaceBlockEntity>,
    calls: AtomicUsize,
}

impl EventHandler<FurnaceBurnEvent> for FuelSwap {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut FurnaceBurnEvent,
    ) -> BoxFuture<'a, ()> {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                event.cancelled = true;
                self.furnace.set_stack(1, ItemStack::new(1, &Item::COAL));
            }
            1 => self
                .furnace
                .set_stack(1, ItemStack::new(1, &Item::CHARCOAL)),
            2 => event.burn_time = 20,
            _ => {}
        }
        Box::pin(async {})
    }
}

struct StartSwap {
    furnace: Arc<FurnaceBlockEntity>,
    calls: AtomicUsize,
}

impl EventHandler<FurnaceStartSmeltEvent> for StartSwap {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut FurnaceStartSmeltEvent,
    ) -> BoxFuture<'a, ()> {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                event.cancelled = true;
                self.furnace
                    .set_stack(1, ItemStack::new(1, &Item::LAVA_BUCKET));
            }
            1 => event.cooking_time = 5,
            _ => {}
        }
        Box::pin(async {})
    }
}

struct CancelFirstSmelt(AtomicUsize);

impl EventHandler<FurnaceSmeltEvent> for CancelFirstSmelt {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut FurnaceSmeltEvent,
    ) -> BoxFuture<'a, ()> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g01_event_cancel_adjustment_and_callback_source_revalidation() {
    run(|fixture| {
        let world = fixture.world();
        prepare_world(&world);
        let furnace = furnace(&world, BlockPos::new(8, 64, 8));
        furnace.set_stack(0, ItemStack::new(1, &Item::RAW_IRON));
        furnace.set_stack(1, ItemStack::new(1, &Item::LAVA_BUCKET));

        fixture
            .server
            .plugin_manager
            .register::<FurnaceBurnEvent, _>(
                Arc::new(FuelSwap {
                    furnace: furnace.clone(),
                    calls: AtomicUsize::new(0),
                }),
                EventPriority::Normal,
                true,
            );
        fixture
            .server
            .plugin_manager
            .register::<FurnaceStartSmeltEvent, _>(
                Arc::new(StartSwap {
                    furnace: furnace.clone(),
                    calls: AtomicUsize::new(0),
                }),
                EventPriority::Normal,
                true,
            );
        fixture
            .server
            .plugin_manager
            .register::<FurnaceSmeltEvent, _>(
                Arc::new(CancelFirstSmelt(AtomicUsize::new(0))),
                EventPriority::Normal,
                true,
            );

        tick(fixture, 1);
        assert_eq!(furnace.get_stack(1).item, &Item::COAL);
        assert!(!furnace.is_burning(), "cancelled fuel must not be consumed");

        tick(fixture, 1);
        assert_eq!(furnace.get_stack(1).item, &Item::CHARCOAL);
        assert!(
            !furnace.is_burning(),
            "callback replacement must not be consumed"
        );

        tick(fixture, 1);
        assert!(furnace.is_burning());
        assert_eq!(furnace.get_lit_time_remaining(), 20);
        assert_eq!(furnace.get_stack(1).item, &Item::LAVA_BUCKET);
        assert_eq!(furnace.get_cooking_time_spent(), 0);

        tick(fixture, 5);
        assert_eq!(furnace.get_stack(0).item, &Item::RAW_IRON);
        assert_eq!(furnace.get_stack(0).item_count, 1);
        assert_eq!(furnace.get_stack(2).item_count, 0);
        assert_eq!(furnace.get_cooking_time_spent(), 0);
        assert_eq!(furnace.get_stack(1).item, &Item::LAVA_BUCKET);
        assert_eq!(furnace.get_stack(1).item_count, 1);
    })
    .await;
}
