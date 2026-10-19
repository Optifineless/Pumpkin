use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use super::*;
use crate::{
    block::OnScheduledTickArgs,
    entity::{
        Entity, EntityBase, area_effect_cloud::AreaEffectCloudEntity,
        death_test_world::DeathTestWorld, item::ItemEntity,
    },
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    biome::Biome, effect::StatusEffect, entity::EntityType, item::Item, item_stack::ItemStack,
};
use pumpkin_util::{GameMode, math::vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use rustc_hash::FxHashSet;

fn add_chunk(world: &Arc<World>) {
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
}

fn put_plate(world: &Arc<World>, block: &Block, position: &BlockPos) {
    world.set_block_state(position, block.default_state.id, BlockFlags::FORCE_STATE);
}

fn spawn_dirt_item(world: &Arc<World>, position: &BlockPos, no_physics: bool) -> Arc<ItemEntity> {
    let entity = Entity::new(
        world.clone(),
        Vector3::new(
            f64::from(position.0.x) + 0.5,
            f64::from(position.0.y),
            f64::from(position.0.z) + 0.5,
        ),
        &EntityType::ITEM,
    );
    entity.no_physics.store(no_physics, Relaxed);
    let item = Arc::new(ItemEntity::new(entity, ItemStack::new(1, &Item::DIRT)));
    assert!(world.spawn_entity(item.clone()));
    item
}

fn collide(fixture: &DeathTestWorld, entity: &dyn EntityBase) {
    Entity::check_block_collision(entity, &fixture.server);
}

#[expect(
    clippy::unwrap_used,
    reason = "The fixture installs the registered pressure-plate behaviors and ticks"
)]
fn run_pressure_plate_ticks(fixture: &DeathTestWorld, ticks: usize) {
    let world = fixture.world();
    let active_chunks = FxHashSet::from_iter([Vector2::new(0, 0)]);
    for _ in 0..ticks {
        let game_time = {
            let mut time = world
                .level_time
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            time.tick(world.level_info.load().game_rules.advance_time);
            time.world_age
        };
        world.level.set_game_time(game_time);
        let tick_data = world.level.get_tick_data(&active_chunks, 0);
        for scheduled in tick_data.block_ticks {
            let block = world.get_block(&scheduled.position);
            if block == scheduled.value {
                world
                    .block_registry
                    .get_pumpkin_block(block.id)
                    .unwrap()
                    .on_scheduled_tick(OnScheduledTickArgs {
                        world: &world,
                        block,
                        position: &scheduled.position,
                    });
            }
        }
    }
}

fn power(world: &World, block: &Block, position: &BlockPos) -> u8 {
    world.block_registry.get_weak_redstone_power(
        block,
        world,
        position,
        world.get_block_state(position),
        BlockDirection::Up,
    )
}

#[test]
fn weighted_plates_scale_with_entity_count() {
    use super::weighted::signal_strength;

    assert_eq!(signal_strength(0, 15), 0);
    assert_eq!(signal_strength(1, 15), 1);
    assert_eq!(signal_strength(7, 15), 7);
    assert_eq!(signal_strength(20, 15), 15);
    assert_eq!(signal_strength(1, 150), 1);
    assert_eq!(signal_strength(75, 150), 8);
}

#[test]
fn detection_box_is_offset_to_block_position() {
    let bounding_box = detection_box_at(&BlockPos::new(10, 64, -5));

    assert_eq!(bounding_box.min, Vector3::new(10.0625, 64.0, -4.9375));
    assert_eq!(bounding_box.max, Vector3::new(10.9375, 64.25, -4.0625));
}

#[test]
fn entity_at_plate_center_intersects_detection_box() {
    let detection_box = detection_box_at(&BlockPos::new(0, 0, 0));
    let entity_box = BoundingBox::new_array([0.4, 0.0, 0.4], [0.6, 1.8, 0.6]);

    assert!(detection_box.intersects(&entity_box));
}

#[test]
fn entity_outside_horizontal_bounds_does_not_intersect_detection_box() {
    let detection_box = detection_box_at(&BlockPos::new(0, 0, 0));
    let entity_box = BoundingBox::new_array([0.95, 0.0, 0.4], [1.0, 0.2, 0.6]);

    assert!(!detection_box.intersects(&entity_box));
}

#[test]
fn entity_above_detection_height_does_not_intersect_detection_box() {
    let detection_box = detection_box_at(&BlockPos::new(0, 0, 0));
    let entity_box = BoundingBox::new_array([0.4, 0.3, 0.4], [0.6, 0.6, 0.6]);

    assert!(!detection_box.intersects(&entity_box));
}

#[test]
fn entity_partially_overlapping_detection_box_intersects() {
    let detection_box = detection_box_at(&BlockPos::new(0, 0, 0));
    let entity_box = BoundingBox::new_array([0.9, 0.2, 0.4], [0.95, 0.3, 0.6]);

    assert!(detection_box.intersects(&entity_box));
}

#[test]
fn entity_touching_detection_box_boundary_does_not_intersect() {
    let detection_box = detection_box_at(&BlockPos::new(0, 0, 0));
    let entity_box = BoundingBox::new_array([0.9375, 0.0, 0.4], [1.0, 0.2, 0.6]);

    assert!(!detection_box.intersects(&entity_box));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_ground_item_does_not_activate_stone_plate() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let stone_pos = BlockPos::new(4, 64, 8);
    put_plate(&world, &Block::STONE_PRESSURE_PLATE, &stone_pos);
    let item = spawn_dirt_item(&world, &stone_pos, false);
    collide(&fixture, item.as_ref());

    assert_eq!(power(&world, &Block::STONE_PRESSURE_PLATE, &stone_pos), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_wooden_and_weighted_item_controls() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let wood_pos = BlockPos::new(8, 64, 8);
    let gold_pos = BlockPos::new(12, 64, 8);
    let iron_pos = BlockPos::new(10, 64, 8);
    let saturated_pos = BlockPos::new(13, 64, 8);
    let plates = [
        (wood_pos, &Block::OAK_PRESSURE_PLATE),
        (gold_pos, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE),
        (iron_pos, &Block::HEAVY_WEIGHTED_PRESSURE_PLATE),
        (saturated_pos, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE),
    ];
    for (position, block) in plates {
        put_plate(&world, block, &position);
    }

    let wood_item = spawn_dirt_item(&world, &wood_pos, false);
    let gold_item = spawn_dirt_item(&world, &gold_pos, false);
    let iron_item = spawn_dirt_item(&world, &iron_pos, false);
    let saturated_items = (0..16)
        .map(|_| spawn_dirt_item(&world, &saturated_pos, false))
        .collect::<Vec<_>>();

    for item in [&wood_item, &gold_item, &iron_item] {
        collide(&fixture, item.as_ref());
    }
    collide(&fixture, saturated_items[0].as_ref());

    assert_eq!(power(&world, &Block::OAK_PRESSURE_PLATE, &wood_pos), 15);
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos),
        1
    );
    assert_eq!(
        power(&world, &Block::HEAVY_WEIGHTED_PRESSURE_PLATE, &iron_pos),
        1
    );
    assert_eq!(
        power(
            &world,
            &Block::LIGHT_WEIGHTED_PRESSURE_PLATE,
            &saturated_pos
        ),
        15
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_stone_plate_recounts_only_living_entities() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let position = BlockPos::new(8, 64, 8);
    put_plate(&world, &Block::STONE_PRESSURE_PLATE, &position);
    let player = fixture.player("stone-plate-survival");
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    collide(&fixture, player.as_ref());
    assert_eq!(power(&world, &Block::STONE_PRESSURE_PLATE, &position), 15);

    let _item = spawn_dirt_item(&world, &position, false);
    player.get_entity().set_pos(Vector3::new(40.5, 64.0, 8.5));
    run_pressure_plate_ticks(&fixture, 20);

    assert_eq!(power(&world, &Block::STONE_PRESSURE_PLATE, &position), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_spectator_does_not_activate_any_pressure_plate() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let stone_pos = BlockPos::new(4, 64, 8);
    let wood_pos = BlockPos::new(8, 64, 8);
    let gold_pos = BlockPos::new(12, 64, 8);
    put_plate(&world, &Block::STONE_PRESSURE_PLATE, &stone_pos);
    put_plate(&world, &Block::OAK_PRESSURE_PLATE, &wood_pos);
    put_plate(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos);

    let spectator = fixture.player("pressure-plate-spectator");
    assert!(spectator.set_gamemode(GameMode::Spectator));
    for (position, block) in [
        (stone_pos, &Block::STONE_PRESSURE_PLATE),
        (wood_pos, &Block::OAK_PRESSURE_PLATE),
        (gold_pos, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE),
    ] {
        spectator.get_entity().set_pos(Vector3::new(
            f64::from(position.0.x) + 0.5,
            f64::from(position.0.y),
            f64::from(position.0.z) + 0.5,
        ));
        collide(&fixture, spectator.as_ref());
        assert_eq!(power(&world, block, &position), 0);
    }

    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_weighted_recount_excludes_spectators() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let gold_pos = BlockPos::new(12, 64, 8);
    put_plate(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos);
    let player = fixture.player("pressure-plate-survival");
    player.get_entity().set_pos(Vector3::new(12.5, 64.0, 8.5));
    collide(&fixture, player.as_ref());
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos),
        1
    );

    let spectator = fixture.player("pressure-plate-spectator");
    assert!(spectator.set_gamemode(GameMode::Spectator));
    spectator
        .get_entity()
        .set_pos(Vector3::new(12.5, 64.0, 8.5));
    run_pressure_plate_ticks(&fixture, 10);
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos),
        1
    );

    player.get_entity().set_pos(Vector3::new(40.5, 64.0, 8.5));
    run_pressure_plate_ticks(&fixture, 10);
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &gold_pos),
        0
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_no_physics_entities_do_not_activate_plates() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let stone_pos = BlockPos::new(4, 64, 8);
    let wood_pos = BlockPos::new(8, 64, 8);
    let gold_pos = BlockPos::new(12, 64, 8);
    for (position, block) in [
        (stone_pos, &Block::STONE_PRESSURE_PLATE),
        (wood_pos, &Block::OAK_PRESSURE_PLATE),
        (gold_pos, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE),
    ] {
        put_plate(&world, block, &position);
    }

    let no_physics_item = spawn_dirt_item(&world, &stone_pos, true);
    for (position, block) in [
        (stone_pos, &Block::STONE_PRESSURE_PLATE),
        (wood_pos, &Block::OAK_PRESSURE_PLATE),
        (gold_pos, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE),
    ] {
        no_physics_item.get_entity().set_pos(Vector3::new(
            f64::from(position.0.x) + 0.5,
            f64::from(position.0.y),
            f64::from(position.0.z) + 0.5,
        ));
        assert!(
            !no_physics_item
                .get_entity()
                .tick_block_collisions(no_physics_item.as_ref())
        );
        assert_eq!(power(&world, block, &position), 0);
    }
    no_physics_item.get_entity().remove();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_no_physics_entity_is_counted_during_recount() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let ignored_pos = BlockPos::new(4, 64, 8);
    put_plate(&world, &Block::OAK_PRESSURE_PLATE, &ignored_pos);
    let ignored_item = spawn_dirt_item(&world, &ignored_pos, false);
    collide(&fixture, ignored_item.as_ref());
    assert_eq!(power(&world, &Block::OAK_PRESSURE_PLATE, &ignored_pos), 15);
    ignored_item.get_entity().no_physics.store(true, Relaxed);
    run_pressure_plate_ticks(&fixture, 20);
    assert_eq!(power(&world, &Block::OAK_PRESSURE_PLATE, &ignored_pos), 15);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_area_effect_cloud_recounts_active_plate() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let plate_pos = BlockPos::new(4, 64, 8);
    put_plate(&world, &Block::OAK_PRESSURE_PLATE, &plate_pos);
    let player = fixture.player("pressure-plate-cloud-survival");
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 8.5));
    collide(&fixture, player.as_ref());
    assert_eq!(power(&world, &Block::OAK_PRESSURE_PLATE, &plate_pos), 15);

    let cloud = AreaEffectCloudEntity::create(
        Entity::new(
            world.clone(),
            Vector3::new(5.5, 64.0, 8.5),
            &EntityType::AREA_EFFECT_CLOUD,
        ),
        ItemStack::new(1, &Item::LINGERING_POTION),
        vec![(&StatusEffect::POISON, 100, 0, false, true, true)],
        600,
        3.0,
        20,
        10,
        -0.5,
        0,
    );
    assert!(world.spawn_entity(cloud.clone()));
    assert!(detection_box_at(&plate_pos).intersects(&cloud.entity.bounding_box.load()));
    player.get_entity().set_pos(Vector3::new(40.5, 64.0, 8.5));

    run_pressure_plate_ticks(&fixture, 20);

    assert!(!cloud.entity.is_removed());
    assert_eq!(power(&world, &Block::OAK_PRESSURE_PLATE, &plate_pos), 15);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g13_removed_item_recounts_weighted_plate_to_zero() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);

    let removed_pos = BlockPos::new(12, 64, 8);
    put_plate(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &removed_pos);
    let removed_item = spawn_dirt_item(&world, &removed_pos, false);
    collide(&fixture, removed_item.as_ref());
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &removed_pos),
        1
    );
    removed_item.get_entity().remove();
    run_pressure_plate_ticks(&fixture, 10);
    assert_eq!(
        power(&world, &Block::LIGHT_WEIGHTED_PRESSURE_PLATE, &removed_pos),
        0
    );
    fixture.server.shutdown().await;
}
