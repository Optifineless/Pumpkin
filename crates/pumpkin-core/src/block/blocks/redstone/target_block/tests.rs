#![expect(
    clippy::unwrap_used,
    reason = "The focused regression fixture requires successful setup"
)]

use super::*;
use crate::{
    block::blocks::redstone::get_redstone_power,
    entity::{
        Entity,
        death_test_world::DeathTestWorld,
        projectile::{arrow::ArrowEntity, snowball::SnowballEntity, trident::TridentEntity},
    },
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::{RedstoneWireLikeProperties, TargetProperties},
    entity::EntityType,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum TestProjectile {
    Arrow,
    SpectralArrow,
    Trident,
    Snowball,
}

impl TestProjectile {
    const fn entity_type(self) -> &'static EntityType {
        match self {
            Self::Arrow => &EntityType::ARROW,
            Self::SpectralArrow => &EntityType::SPECTRAL_ARROW,
            Self::Trident => &EntityType::TRIDENT,
            Self::Snowball => &EntityType::SNOWBALL,
        }
    }
}

fn prepare_world(fixture: &DeathTestWorld) -> Arc<World> {
    let world = fixture.world();
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::DIRT),
    );
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    world.update_active_chunks();
    world
}

fn place(world: &Arc<World>, position: &BlockPos, block: &Block) {
    world.set_block_state(position, block.default_state.id, BlockFlags::NOTIFY_ALL);
}

fn projectile_hit(
    world: &Arc<World>,
    fixture: &DeathTestWorld,
    kind: &'static EntityType,
    target: &BlockPos,
    x_offset: f64,
) {
    let start = Vector3::new(
        f64::from(target.0.x) + x_offset,
        f64::from(target.0.y) + 0.5,
        f64::from(target.0.z) + 2.5,
    );
    let entity = Entity::new(world.clone(), start, kind);
    let projectile: Arc<dyn EntityBase> =
        if kind == &EntityType::ARROW || kind == &EntityType::SPECTRAL_ARROW {
            let arrow = ArrowEntity::new(entity, None);
            arrow.entity.velocity.store(Vector3::new(0.0, 0.0, -3.0));
            Arc::new(arrow)
        } else if kind == &EntityType::TRIDENT {
            let trident = TridentEntity::new(entity, None);
            trident.entity.velocity.store(Vector3::new(0.0, 0.0, -3.0));
            Arc::new(trident)
        } else {
            let snowball = SnowballEntity::new(entity);
            snowball
                .get_entity()
                .velocity
                .store(Vector3::new(0.0, 0.0, -3.0));
            Arc::new(snowball)
        };
    projectile.tick(projectile.as_ref(), &fixture.server);
}

fn target_power(world: &World, position: &BlockPos) -> u8 {
    TargetProperties::from_state_id(world.get_block_state_id(position)).r#power
}

fn advance_ticks(fixture: &DeathTestWorld, world: &Arc<World>, count: usize) {
    for _ in 0..count {
        world.update_active_chunks();
        world.tick_environment();
        world.tick_chunks(&fixture.server);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_block_b1019_g12_keeps_weak_output_without_strong_conduction() {
    let fixture = DeathTestWorld::new().await;
    let world = prepare_world(&fixture);
    let center = BlockPos::new(4, 64, 8);
    let edge = BlockPos::new(10, 64, 8);
    place(&world, &center, &Block::TARGET);
    place(&world, &edge, &Block::TARGET);

    // Both impacts enter through the north face; the second is near the west edge.
    projectile_hit(&world, &fixture, &EntityType::ARROW, &center, 0.5);
    projectile_hit(&world, &fixture, &EntityType::SPECTRAL_ARROW, &edge, 0.1);
    assert_eq!(target_power(&world, &center), 15, "center hit control");
    assert_eq!(target_power(&world, &edge), 3, "edge hit control");

    let state = world.get_block_state(&center);
    for direction in BlockDirection::all() {
        assert_eq!(
            world.block_registry.get_weak_redstone_power(
                &Block::TARGET,
                &world,
                &center,
                state,
                direction,
            ),
            15,
            "target weak output for {direction:?}"
        );
        assert_eq!(
            world.block_registry.get_strong_redstone_power(
                &Block::TARGET,
                &world,
                &center,
                state,
                direction,
            ),
            0,
            "target direct output for {direction:?}"
        );
    }

    let stone = center.east();
    let wire = stone.east();
    place(&world, &stone, &Block::STONE);
    place(&world, &wire, &Block::REDSTONE_WIRE);
    assert_eq!(
        get_redstone_power(
            &Block::STONE,
            world.get_block_state(&stone),
            &world,
            &stone,
            BlockDirection::East,
        ),
        0,
        "the solid block between the target and wire must not conduct target power"
    );
    assert_eq!(
        RedstoneWireLikeProperties::from_state_id(world.get_block_state_id(&wire)).power,
        0,
        "wire beyond the solid block must remain unpowered"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_block_b1019_g12_uses_arrow_family_pulse_and_first_hit_deadline() {
    let fixture = DeathTestWorld::new().await;
    let world = prepare_world(&fixture);
    let cases = [
        (TestProjectile::Arrow, BlockPos::new(2, 64, 3), 20),
        (TestProjectile::SpectralArrow, BlockPos::new(5, 64, 3), 20),
        (TestProjectile::Trident, BlockPos::new(8, 64, 3), 20),
        (TestProjectile::Snowball, BlockPos::new(11, 64, 3), 8),
    ];
    for (kind, position, _) in cases {
        place(&world, &position, &Block::TARGET);
        projectile_hit(&world, &fixture, kind.entity_type(), &position, 0.5);
        assert_eq!(target_power(&world, &position), 15, "{kind:?}");
    }

    advance_ticks(&fixture, &world, 6);
    projectile_hit(&world, &fixture, &EntityType::ARROW, &cases[0].1, 0.5);
    assert!(world.is_block_tick_scheduled(&cases[0].1, &Block::TARGET));

    advance_ticks(&fixture, &world, 2);
    for (kind, position, duration) in cases {
        assert_eq!(
            target_power(&world, &position),
            if duration == 20 { 15 } else { 0 },
            "{kind:?} pulse after eight ticks"
        );
    }

    advance_ticks(&fixture, &world, 12);
    for (kind, position, _) in cases {
        assert_eq!(
            target_power(&world, &position),
            0,
            "{kind:?} pulse after 20 ticks"
        );
    }
    fixture.server.shutdown().await;
}
