#![expect(
    clippy::unwrap_used,
    reason = "Real furnace fixtures must construct and save"
)]

use std::{future::Future, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use futures::FutureExt;
use pumpkin_data::{Block, biome::Biome, dimension::Dimension, item::Item};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;

use super::CookingBlockEntityBase;
use crate::{
    block::entities::{
        BlockEntity, blasting_furnace::BlastingFurnaceBlockEntity, block_entity_from_nbt,
        furnace::FurnaceBlockEntity, smoker::SmokerBlockEntity,
    },
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::inventory::{FurnaceBurnEvent, FurnaceStartSmeltEvent},
    },
    server::{
        Server,
        scenario::{Scenario, run as scenario_run},
    },
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

const TIMERS: [&str; 4] = [
    "cooking_time_spent",
    "cooking_total_time",
    "lit_time_remaining",
    "lit_total_time",
];
const APPLIANCES: [(&Block, &str, &str); 3] = [
    (
        &Block::FURNACE,
        "minecraft:raw_iron",
        "minecraft:iron_ingot",
    ),
    (
        &Block::BLAST_FURNACE,
        "minecraft:iron_ore",
        "minecraft:iron_ingot",
    ),
    (&Block::SMOKER, "minecraft:potato", "minecraft:baked_potato"),
];

async fn run<F: Future<Output = ()>>(action: impl FnOnce(Arc<World>) -> F) {
    let fixture = tokio::time::timeout(Duration::from_secs(90), DeathTestWorld::new())
        .await
        .unwrap();
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let result = AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), action(world)))
        .catch_unwind()
        .await;
    tokio::time::timeout(Duration::from_secs(90), fixture.server.shutdown())
        .await
        .unwrap();
    match result {
        Ok(result) => result.unwrap(),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn record(block: &Block, x: i32, timers: [i32; 4], input: &str) -> NbtCompound {
    // Literal AbstractFurnaceBlockEntity.saveAdditional schema, independent of our writer.
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", format!("minecraft:{}", block.name));
    nbt.put_int("x", x);
    nbt.put_int("y", 64);
    nbt.put_int("z", 8);
    for (key, value) in TIMERS.into_iter().zip(timers) {
        nbt.put_int(key, value);
    }
    nbt.put_float("speed_multiplier", 2.0);
    let mut item = NbtCompound::new();
    item.put_byte("Slot", 0);
    item.put_string("id", input.to_string());
    item.put_int("count", 2);
    nbt.put_list("Items", vec![NbtTag::Compound(item)]);
    let mut recipes = NbtCompound::new();
    recipes.put_int("minecraft:iron_ingot_from_smelting_raw_iron", 3);
    nbt.put_compound("RecipesUsed", recipes);
    nbt
}

fn legacy_record(block: &Block, x: i32, input: &str) -> NbtCompound {
    let mut nbt = record(block, x, [0; 4], input);
    // Old fork AtomicU16 values 40000 and 50000 were written with these signed bits.
    for (key, value) in TIMERS.into_iter().zip([197, 200, -25_536, -15_536]) {
        nbt.put_short(key, value);
    }
    nbt
}

fn load(world: &Arc<World>, block: &Block, nbt: &NbtCompound) -> Arc<dyn BlockEntity> {
    let position = BlockPos::new(nbt.get_int("x").unwrap(), 64, 8);
    world.set_block_state(&position, block.default_state.id, BlockFlags::FORCE_STATE);
    // Placement installs an empty live entity; imported NBT must enter the lazy loader.
    world.remove_block_entity(&position);
    world.add_block_entity_nbt(position, nbt);
    world.get_block_entity(&position).unwrap()
}

fn cooking(entity: &Arc<dyn BlockEntity>) -> &dyn CookingBlockEntityBase {
    if let Some(entity) = entity.as_any().downcast_ref::<FurnaceBlockEntity>() {
        entity
    } else if let Some(entity) = entity.as_any().downcast_ref::<BlastingFurnaceBlockEntity>() {
        entity
    } else {
        entity.as_any().downcast_ref::<SmokerBlockEntity>().unwrap()
    }
}

fn timers(entity: &Arc<dyn BlockEntity>) -> [i32; 4] {
    let delegate = entity.clone().to_property_delegate().unwrap();
    [2, 3, 0, 1].map(|index| delegate.get_property(index))
}

fn tick(world: &Arc<World>) {
    world.tick(&world.server.upgrade().unwrap());
}

fn saved(world: &World, entity: &Arc<dyn BlockEntity>) -> NbtCompound {
    let position = entity.get_position();
    world
        .level
        .read_chunk_sync(&position.chunk_position(), |chunk| {
            chunk
                .pending_block_entities
                .lock()
                .unwrap()
                .get(&position)
                .unwrap()
                .clone()
        })
        .unwrap()
}

fn assert_ints(nbt: &NbtCompound, expected: [i32; 4]) {
    for (key, value) in TIMERS.into_iter().zip(expected) {
        assert_eq!(
            nbt.get(key),
            Some(&NbtTag::Int(value)),
            "canonical Int {key}"
        );
    }
}

#[tokio::test]
async fn b1020c_g08_furnace_int_import_continues_cooking() {
    run(|world| async move {
        for (index, (block, input, output)) in APPLIANCES.into_iter().enumerate() {
            let nbt = record(
                block,
                8 + index as i32,
                [70_000, 70_002, 80_000, 90_000],
                input,
            );
            let entity = load(&world, block, &nbt);
            assert_eq!(
                timers(&entity),
                [70_000, 70_002, 80_000, 90_000],
                "Int import"
            );
            assert_eq!(cooking(&entity).get_speed_multiplier(), 2.0);
            tick(&world);
            assert_eq!(timers(&entity), [70_001, 70_002, 79_999, 90_000]);
            world.save().await;
            let written = saved(&world, &entity);
            assert_ints(&written, [70_001, 70_002, 79_999, 90_000]);
            assert_eq!(
                written.get_compound("RecipesUsed"),
                nbt.get_compound("RecipesUsed")
            );
            tick(&world);
            let inventory = entity.clone().get_inventory().unwrap();
            assert_eq!(inventory.get_stack(0).item_count, 1);
            assert_eq!(
                inventory.get_stack(2).item.registry_key,
                output.strip_prefix("minecraft:").unwrap()
            );
            assert_eq!(inventory.get_stack(2).item_count, 1);
            assert!(
                inventory.get_stack(1).is_empty(),
                "imported heat needs no new fuel"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn b1020c_g08_furnace_export_is_canonical_int() {
    run(|world| async move {
        for (index, (block, input, _)) in APPLIANCES.into_iter().enumerate() {
            let entity = load(
                &world,
                block,
                &legacy_record(block, 8 + index as i32, input),
            );
            assert_eq!(
                timers(&entity),
                [197, 200, 40_000, 50_000],
                "legacy load control"
            );
            entity.clone().get_inventory().unwrap().mark_dirty();
            tick(&world);
            world.save().await;
            let expected = [198, 200, 39_999, 50_000];
            assert_ints(&saved(&world, &entity), expected);
            assert_ints(&entity.chunk_data_nbt().unwrap(), expected);
            let mut normal = NbtCompound::new();
            entity.write_internal(&mut normal);
            assert_ints(&normal, expected);
            assert_eq!(normal.get_float("speed_multiplier"), Some(2.0));
            assert_eq!(cooking(&entity).get_input_item().item_count, 2);
        }
    })
    .await;
}

#[tokio::test]
async fn b1020c_g08_furnace_missing_fields_and_legacy_controls() {
    run(|world| async move {
        let mut missing = record(&Block::FURNACE, 8, [1; 4], "minecraft:raw_iron");
        for key in TIMERS {
            missing.child_tags.remove(key);
        }
        missing.child_tags.remove("Items");
        missing.child_tags.remove("speed_multiplier");
        let empty = load(&world, &Block::FURNACE, &missing);
        assert_eq!(timers(&empty), [0; 4]);
        assert_eq!(cooking(&empty).get_speed_multiplier(), 1.0);
        assert!(empty.clone().get_inventory().unwrap().is_empty());
        tick(&world);
        assert_eq!(timers(&empty), [0; 4]);
        for (index, (block, input, _)) in APPLIANCES.into_iter().enumerate() {
            let nbt = legacy_record(block, 9 + index as i32, input);
            let factory = block_entity_from_nbt(&nbt).unwrap();
            assert_eq!(timers(&factory), [197, 200, 40_000, 50_000]);
            let entity = load(&world, block, &nbt);
            for _ in 0..3 {
                tick(&world);
            }
            let inventory = entity.clone().get_inventory().unwrap();
            assert_eq!(inventory.get_stack(0).item_count, 1);
            assert_eq!(inventory.get_stack(2).item_count, 1);
            assert!(inventory.get_stack(1).is_empty());
        }
    })
    .await;
}

#[tokio::test]
async fn b1020c_g08_furnace_signed_tick_controls() {
    run(|world| async move {
        // Supplied serverTick: only positive idle progress cools; Mth.clamp's max may be negative.
        for (x, before, after) in [
            (8, [1, 200, 0, 7], [0, 200, 0, 7]),
            (9, [-3, 200, -5, -7], [-3, 200, -5, -7]),
            (10, [1, -4, 0, 7], [-4, -4, 0, 7]),
        ] {
            let mut nbt = record(&Block::FURNACE, x, before, "minecraft:raw_iron");
            nbt.child_tags.remove("Items");
            let entity = load(&world, &Block::FURNACE, &nbt);
            tick(&world);
            assert_eq!(timers(&entity), after, "signed idle serverTick at {x}");
        }
        for (x, before, after, outputs) in [
            (11, [-3, 200, 10, 11], [-2, 200, 9, 11], 0),
            (12, [0, -4, 10, 11], [0, 100, 9, 11], 1),
            (
                13,
                [i32::MAX, i32::MAX, 10, 11],
                [i32::MIN, i32::MAX, 9, 11],
                0,
            ),
        ] {
            let entity = load(
                &world,
                &Block::FURNACE,
                &record(&Block::FURNACE, x, before, "minecraft:raw_iron"),
            );
            tick(&world);
            assert_eq!(timers(&entity), after, "signed lit serverTick at {x}");
            assert_eq!(cooking(&entity).get_output_item().item_count, outputs);
        }
    })
    .await;
}

#[tokio::test]
async fn b1020c_g08_furnace_property_and_scaled_duration() {
    run(|world| async move {
        let entity = load(
            &world,
            &Block::FURNACE,
            &record(&Block::FURNACE, 8, [0; 4], "minecraft:raw_iron"),
        );
        let delegate = entity.clone().to_property_delegate().unwrap();
        for (index, value) in [(2, 70_000), (3, 80_000), (0, -2), (1, 90_000)] {
            delegate.set_property(index, value);
        }
        assert_eq!(timers(&entity), [70_000, 80_000, -2, 90_000]);
        world.save().await;
        assert_ints(&saved(&world, &entity), [70_000, 80_000, -2, 90_000]);
        cooking(&entity).set_speed_multiplier(0.001);
        entity.clone().get_inventory().unwrap().set_stack(
            0,
            pumpkin_data::item_stack::ItemStack::new(1, &Item::IRON_ORE),
        );
        assert_eq!(
            delegate.get_property(3),
            200_000,
            "scaled recipe duration must retain i32"
        );
        delegate.set_property(2, -100);
        entity
            .clone()
            .get_inventory()
            .unwrap()
            .set_stack(1, pumpkin_data::item_stack::ItemStack::new(1, &Item::COAL));
        tick(&world);
        assert_eq!(delegate.get_property(3), 200);
        assert_eq!(
            delegate.get_property(2),
            1,
            "negative ratio rounds up before normal cook tick"
        );
    })
    .await;
}

struct WideDurations;

impl EventHandler<FurnaceBurnEvent> for WideDurations {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut FurnaceBurnEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(
            event.burn_time, 1600,
            "burn callback keeps its fuel payload"
        );
        event.burn_time = if event.block_pos.0.x == 8 {
            70_000
        } else {
            u32::MAX
        };
        Box::pin(async {})
    }
}

impl EventHandler<FurnaceStartSmeltEvent> for WideDurations {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut FurnaceStartSmeltEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(
            event.cooking_time, 200,
            "start callback keeps its recipe payload"
        );
        event.cooking_time = 70_002;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g08_furnace_event_duration_controls() {
    run(|world| async move {
        let server = world.server.upgrade().unwrap();
        server.plugin_manager.register::<FurnaceBurnEvent, _>(
            Arc::new(WideDurations),
            EventPriority::Normal,
            true,
        );
        server.plugin_manager.register::<FurnaceStartSmeltEvent, _>(
            Arc::new(WideDurations),
            EventPriority::Normal,
            true,
        );
        let entities: Vec<_> = (8..10)
            .map(|x| {
                let entity = load(
                    &world,
                    &Block::FURNACE,
                    &record(&Block::FURNACE, x, [0, 200, 0, 0], "minecraft:raw_iron"),
                );
                entity
                    .clone()
                    .get_inventory()
                    .unwrap()
                    .set_stack(1, pumpkin_data::item_stack::ItemStack::new(1, &Item::COAL));
                entity
            })
            .collect();
        tick(&world);
        assert_eq!(timers(&entities[0]), [1, 70_002, 70_000, 70_000]);
        assert!(cooking(&entities[0]).get_fuel_item().is_empty());
        assert_eq!(timers(&entities[1]), [0, 200, 0, 0]);
        assert_eq!(
            cooking(&entities[1]).get_fuel_item().item_count,
            1,
            "a callback duration outside positive i32 cannot consume fuel"
        );
    })
    .await;
}

#[tokio::test]
async fn b1020c_g08_furnace_legacy_restart_conserves_items() {
    scenario_run(async {
        let scenario = Scenario::new().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario
            .drive(
                "load furnace terrain",
                world
                    .level
                    .get_or_fetch_chunk(Vector2::new(0, 0), Clone::clone),
            )
            .await
            .unwrap();
        for (index, (block, input, _)) in APPLIANCES.into_iter().enumerate() {
            let entity = load(
                &world,
                block,
                &legacy_record(block, 8 + index as i32, input),
            );
            assert_eq!(timers(&entity), [197, 200, 40_000, 50_000]);
        }
        drop(world);
        let scenario = scenario.restart().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario
            .drive(
                "reload saved furnace terrain",
                world
                    .level
                    .get_or_fetch_chunk(Vector2::new(0, 0), Clone::clone),
            )
            .await
            .unwrap();
        world
            .forced_chunks
            .lock()
            .unwrap()
            .insert(Vector2::new(0, 0));
        let entities: Vec<_> = (8..11)
            .map(|x| world.get_block_entity(&BlockPos::new(x, 64, 8)).unwrap())
            .collect();
        for entity in &entities {
            assert_eq!(timers(entity), [197, 200, 40_000, 50_000], "legacy restart");
            assert_eq!(cooking(entity).get_speed_multiplier(), 2.0);
            assert_eq!(cooking(entity).get_input_item().item_count, 2);
            assert_ints(&saved(&world, entity), [197, 200, 40_000, 50_000]);
        }
        for _ in 0..3 {
            tick(&world);
        }
        for entity in &entities {
            assert_eq!(cooking(entity).get_input_item().item_count, 1);
            assert_eq!(cooking(entity).get_output_item().item_count, 1);
            assert!(cooking(entity).get_fuel_item().is_empty());
        }
    })
    .await;
}
