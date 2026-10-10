use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::biome::Biome;
use pumpkin_util::math::vector3::Vector3;
use rand::SeedableRng;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waterloggable_falling_block_lands_waterlogged_in_source_water() {
    use pumpkin_data::block_properties::WaterLikeProperties;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    for (x, landing_state, waterlogged) in [
        (4, Block::WATER.default_state.id, true),
        (8, Block::AIR.default_state.id, false),
        (
            12,
            WaterLikeProperties { level: 1 }.to_state_id(&Block::WATER),
            false,
        ),
    ] {
        let pos = BlockPos::new(x, 64, 4);
        world.set_block_state(&pos, landing_state, BlockFlags::FORCE_STATE);
        let falling = FallingEntity::replace_spawn(
            &world,
            pos.up_height(5),
            Block::OAK_SLAB.default_state.id,
        );
        falling
            .entity
            .set_pos(Vector3::new(f64::from(x) + 0.5, 64.0, 4.5));
        falling.land();
        assert!(falling.entity.is_removed());
        assert_eq!(world.get_block(&pos), &Block::OAK_SLAB);
        assert_eq!(world.get_block_state(&pos).is_waterlogged(), waterlogged);
    }
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falling_anvil_damages_entities_below_and_can_crack() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    let living = cow.get_living_entity().unwrap();
    living.set_max_health(100.0);
    living.set_health(100.0);
    let falling = FallingEntity::replace_spawn(
        &world,
        BlockPos::new(4, 85, 4),
        Block::ANVIL.default_state.id,
    );
    falling.entity.set_pos(Vector3::new(4.5, 64.0, 4.5));
    falling.cause_fall_damage(21.0, &mut rand::rngs::StdRng::seed_from_u64(1));
    assert_eq!(living.health.load(), 60.0);
    falling.land();
    assert_eq!(
        world.get_block(&BlockPos::new(4, 64, 4)),
        &Block::CHIPPED_ANVIL
    );

    let short_cow = fixture.mob(&EntityType::COW);
    short_cow.get_living_entity().unwrap().set_max_health(20.0);
    short_cow.get_living_entity().unwrap().set_health(20.0);
    short_cow.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    let short = FallingEntity::replace_spawn(
        &world,
        BlockPos::new(8, 65, 8),
        Block::ANVIL.default_state.id,
    );
    short.entity.set_pos(Vector3::new(8.5, 64.0, 8.5));
    short.cause_fall_damage(0.9, &mut rand::rngs::StdRng::seed_from_u64(1));
    assert_eq!(short_cow.get_living_entity().unwrap().health.load(), 20.0);
    short.land();
    assert_eq!(world.get_block(&BlockPos::new(8, 64, 8)), &Block::ANVIL);

    let broken = FallingEntity::replace_spawn(
        &world,
        BlockPos::new(12, 85, 12),
        Block::DAMAGED_ANVIL.default_state.id,
    );
    broken.entity.set_pos(Vector3::new(12.5, 64.0, 12.5));
    broken.cause_fall_damage(21.0, &mut rand::rngs::StdRng::seed_from_u64(1));
    broken.land();
    assert_eq!(world.get_block(&BlockPos::new(12, 64, 12)), &Block::AIR);
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falling_anvil_tick_accumulates_distance_and_skips_creative_and_spectator() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let victim = fixture.mob(&EntityType::COW);
    victim.get_living_entity().unwrap().set_max_health(20.0);
    victim.get_living_entity().unwrap().set_health(20.0);
    victim.get_entity().set_pos(Vector3::new(6.5, 64.0, 6.5));
    let creative = fixture.player("creative");
    creative.set_gamemode(pumpkin_util::GameMode::Creative);
    creative.get_entity().set_pos(Vector3::new(6.5, 64.0, 6.5));
    let spectator = fixture.player("spectator");
    spectator.set_gamemode(pumpkin_util::GameMode::Spectator);
    spectator.get_entity().set_pos(Vector3::new(6.5, 64.0, 6.5));
    let falling = FallingEntity::replace_spawn(
        &world,
        BlockPos::new(6, 69, 6),
        Block::ANVIL.default_state.id,
    );
    for _ in 0..80 {
        if falling.entity.is_removed() {
            break;
        }
        falling.tick(falling.as_ref(), &fixture.server);
    }
    assert!(falling.entity.is_removed());
    assert_eq!(victim.get_living_entity().unwrap().health.load(), 12.0);
    assert_eq!(creative.living_entity.health.load(), 20.0);
    assert_eq!(spectator.living_entity.health.load(), 20.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falling_anvil_respects_the_landing_blocks_fall_damage_rule() {
    use pumpkin_data::block_properties::{
        PointedDripstoneLikeProperties, SpeleothemThickness, VerticalDirection,
    };
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut tip = PointedDripstoneLikeProperties::default(&Block::POINTED_DRIPSTONE);
    tip.vertical_direction = VerticalDirection::Up;
    tip.thickness = SpeleothemThickness::Tip;
    for (x, state, sneaking, health) in [
        (4, tip.to_state_id(&Block::POINTED_DRIPSTONE), false, 88.0),
        (8, Block::POWDER_SNOW.default_state.id, false, 100.0),
        (10, Block::RED_BED.default_state.id, false, 98.0),
        (12, Block::SHELF_MUSHROOM.default_state.id, false, 98.0),
        (14, Block::SLIME_BLOCK.default_state.id, true, 100.0),
    ] {
        world.set_block_state(&BlockPos::new(x, 64, 4), state, BlockFlags::FORCE_STATE);
        let cow = fixture.mob(&EntityType::COW);
        cow.get_entity()
            .set_pos(Vector3::new(f64::from(x) + 0.5, 65.0, 4.5));
        let living = cow.get_living_entity().unwrap();
        living.set_max_health(100.0);
        living.set_health(100.0);
        let falling = FallingEntity::replace_spawn(
            &world,
            BlockPos::new(x, 69, 4),
            Block::ANVIL.default_state.id,
        );
        falling
            .entity
            .set_pos(Vector3::new(f64::from(x) + 0.5, 65.0, 4.5));
        falling.entity.set_sneaking(sneaking);
        falling.cause_fall_damage_on_landing(4.0);
        assert_eq!(living.health.load(), health);
    }
    fixture.server.shutdown().await;
}
