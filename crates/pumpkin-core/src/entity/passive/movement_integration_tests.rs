use crate::entity::{
    Entity, EntityBase,
    ai::pathfinder::{navigation_geometry::is_clear_for_movement_between, node::Node, path::Path},
    living::{LivingEntity, test_support::armor_test_world},
    mob::{Mob, elder_guardian::ElderGuardianEntity, guardian::GuardianEntity},
    passive::{axolotl::AxolotlEntity, dolphin::DolphinEntity, strider::StriderEntity},
};
use pumpkin_data::{Block, BlockDirection, attributes::Attributes, entity::EntityType};
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    version::JavaMinecraftVersion,
};
use pumpkin_world::chunk::ChunkData;
use std::sync::{Arc, atomic::Ordering::Relaxed};

fn chunk(world: &crate::world::World) -> Arc<ChunkData> {
    let chunk = ChunkData::empty_sync(0, 0);
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), chunk.clone());
    chunk
}

fn ray(entity: &dyn EntityBase, y: f64) -> bool {
    is_clear_for_movement_between(
        entity,
        Vector3::new(3.5, y, 4.5),
        Vector3::new(5.5, y, 4.5),
        0.0,
        false,
    )
}

#[tokio::test]
async fn zombie_nautilus_loaded_variant_keeps_water_travel() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    chunk(&world);
    let nautilus = crate::entity::r#type::from_type(
        &EntityType::ZOMBIE_NAUTILUS,
        Vector3::new(4.5, 60.0, 4.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    let mob = nautilus.get_mob().unwrap();
    let mut nbt = NbtCompound::new();
    nbt.put_string("variant", "minecraft:warm".to_owned());
    nautilus.read_nbt_non_mut(&nbt);
    nautilus.init_data_tracker();
    // ZombieNautilus inherits AbstractNautilus.travelInWater and isPushedByFluid.
    let entity = nautilus.get_entity();
    entity.touching_water.store(true, Relaxed);
    entity.velocity.store(Vector3::new(0.1, 0.2, 0.3));
    assert!(mob.custom_travel(nautilus.as_ref()));
    assert!((entity.pos.load().y - 60.2).abs() < 1e-8);
    assert!((entity.velocity.load().y - 0.18).abs() < 1e-8);
    assert!(!mob.mob_is_pushed_by_fluids());
    let mut saved = NbtCompound::new();
    mob.mob_write_nbt(&mut saved);
    assert_eq!(saved.get_string("variant"), Some("minecraft:warm"));
}

#[tokio::test]
async fn navigation_ray_uses_scaffold_entity_context() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let chunk = chunk(&world);
    let mut props =
        pumpkin_data::block_properties::ScaffoldingLikeProperties::default(&Block::SCAFFOLDING);
    props.bottom = true;
    props.distance = 1;
    chunk.set_block_absolute_y(4, 60, 4, props.to_state_id(&Block::SCAFFOLDING));
    let living = LivingEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.2, 4.5),
        &EntityType::COW,
    ));
    // The same ray and block change collision solely with the entity context.
    assert!(ray(&living, 60.95));
    assert!(!ray(&living, 60.05));
    living.entity.set_pos(Vector3::new(4.5, 61.0, 4.5));
    assert!(!ray(&living, 60.95));
    living.entity.set_sneaking(true);
    assert!(ray(&living, 60.95));
    living.entity.set_pos(Vector3::new(4.5, 59.99, 4.5));
    assert!(ray(&living, 60.05));
}

#[tokio::test]
async fn navigation_powder_snow_uses_sneaking_and_above_tolerance() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    chunk(&world).set_block_absolute_y(4, 60, 4, Block::POWDER_SNOW.default_state.id);
    let fox = LivingEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.999_995, 4.5),
        &EntityType::FOX,
    ));
    fox.entity.velocity.store(Vector3::new(0.0, -0.1, 0.0));
    assert!(!ray(&fox, 60.5));
    fox.entity.set_sneaking(true);
    assert!(ray(&fox, 60.5));
    fox.fall_distance.store(3.0);
    assert!(!ray(&fox, 60.5));
    assert!(ray(&fox, 60.95));
}

#[tokio::test]
async fn navigation_ray_reads_moving_piston_progress() {
    use crate::block::entities::piston::PistonBlockEntity;
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    chunk(&world).set_block_absolute_y(4, 60, 4, Block::MOVING_PISTON.default_state.id);
    let piston = Arc::new(PistonBlockEntity {
        position: BlockPos::new(4, 60, 4),
        pushed_block_state: Block::STONE.default_state,
        facing: BlockDirection::Up,
        current_progress: 0.0.into(),
        last_progress: 0.0.into(),
        extending: true,
        source: false,
    });
    world
        .block_entities
        .entry(Vector2::new(0, 0))
        .or_default()
        .insert(piston.position, piston.clone());
    let living = LivingEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 61.0, 4.5),
        &EntityType::COW,
    ));
    assert!(ray(&living, 60.75));
    piston.current_progress.store(0.5);
    assert!(!ray(&living, 60.25));
    assert!(ray(&living, 60.75));
    piston.current_progress.store(1.0);
    assert!(!ray(&living, 60.75));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strider_source_support_reaches_movement_before_fall_damage() {
    let fixture = crate::entity::death_test_world::DeathTestWorld::new().await;
    let world = fixture.world();
    let chunk = chunk(&world);
    chunk.set_block_absolute_y(4, 60, 4, Block::LAVA.default_state.id);
    let strider = StriderEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 60.6, 4.5),
        &EntityType::STRIDER,
    ));
    let entity = strider.get_entity();
    let living = &strider.mob_entity.living_entity;
    entity.touching_lava.store(true, Relaxed);
    let motion = Vector3::new(0.0, -0.4, 0.0);
    let bounds = entity.bounding_box.load().stretch(motion);
    let (shapes, positions) = world.get_block_collisions(bounds, strider.as_ref());
    assert_eq!(shapes.len(), 1);
    assert_eq!(positions, [(1, BlockPos::new(4, 60, 4))]);
    assert_eq!(shapes[0].max.y, 60.5);
    let health = living.health.load();
    living.fall_distance.store(20.0);
    entity.move_entity(strider.as_ref(), motion);
    assert_eq!(entity.pos.load().y, 60.5);
    assert!(entity.on_ground.load(Relaxed));
    assert_eq!(living.health.load(), health);
    assert_eq!(living.fall_distance.load(), 0.0);
    // Support must use the original feet, not the bottom of the swept query.
    entity.set_pos(Vector3::new(4.5, 60.2, 4.5));
    assert!(
        world
            .get_block_collisions(bounds, strider.as_ref())
            .0
            .is_empty()
    );
    entity.set_pos(Vector3::new(4.5, 60.6, 4.5));
    chunk.set_block_absolute_y(4, 61, 4, Block::LAVA.default_state.id);
    assert!(
        world
            .get_block_collisions(bounds, strider.as_ref())
            .0
            .is_empty()
    );
    chunk.set_block_absolute_y(4, 61, 4, Block::AIR.default_state.id);
    let mut flowing = pumpkin_data::block_properties::WaterLikeProperties::default(&Block::LAVA);
    flowing.level = 1;
    chunk.set_block_absolute_y(4, 60, 4, flowing.to_state_id(&Block::LAVA));
    assert!(
        world
            .get_block_collisions(bounds, strider.as_ref())
            .0
            .is_empty()
    );
    chunk.set_block_absolute_y(4, 60, 4, Block::LAVA.default_state.id);
    let cow = LivingEntity::new(Entity::new(world, entity.pos.load(), &EntityType::COW));
    assert!(
        cow.entity
            .world
            .load()
            .get_block_collisions(bounds, &cow)
            .0
            .is_empty()
    );
}

#[tokio::test]
async fn axolotl_death_refills_air_and_publishes_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let axolotl = AxolotlEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::AXOLOTL,
    ));
    axolotl.air_supply.store(1, Relaxed);
    axolotl.after_base_tick();
    assert_eq!(axolotl.air_supply.load(Relaxed), 0);
    axolotl.mob_entity.living_entity.health.store(0.0);
    assert!(axolotl.get_entity().is_alive()); // Still present during death animation.
    axolotl.get_entity().synched_data.clear_dirty();
    axolotl.after_base_tick();
    assert_eq!(axolotl.air_supply.load(Relaxed), 6000);
    assert_eq!(
        axolotl
            .get_entity()
            .synched_data
            .pack_dirty_for_version(&JavaMinecraftVersion::V_26_3)
            .unwrap()
            .as_ref(),
        &[1, 1, 0xf0, 0x2e, 0xff]
    );
}

#[tokio::test]
async fn axolotl_air_reads_numeric_nbt() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let axolotl = AxolotlEntity::new(Entity::new(world, Vector3::default(), &EntityType::AXOLOTL));
    for (tag, expected) in [
        (NbtTag::Byte(7), 7),
        (NbtTag::Short(310), 310),
        (NbtTag::Int(50000), 50000),
        (NbtTag::Long(4_294_967_297), 1),
        (NbtTag::Float(-1.2), -2),
        (NbtTag::Double(12.9), 12),
    ] {
        let mut nbt = NbtCompound::new();
        nbt.put("Air", tag);
        axolotl.mob_read_nbt(&nbt);
        assert_eq!(axolotl.air_supply.load(Relaxed), expected);
    }
    axolotl.mob_read_nbt(&NbtCompound::new());
    assert_eq!(axolotl.air_supply.load(Relaxed), 6000);
    // Java int arithmetic wraps even when Air was loaded from an extreme numeric tag.
    let mut nbt = NbtCompound::new();
    nbt.put_int("Air", i32::MIN);
    axolotl.mob_read_nbt(&nbt);
    axolotl.after_base_tick();
    assert_eq!(axolotl.air_supply.load(Relaxed), i32::MAX);
}

#[tokio::test]
async fn aquatic_rain_reaches_head_above_sheltered_feet() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let chunk = chunk(&world);
    chunk.set_block_absolute_y(4, 60, 4, Block::STONE.default_state.id);
    // Supply the exposed column explicitly; this test does not run chunk generation or lighting.
    chunk.heightmap.lock().unwrap().set(
        pumpkin_world::chunk::ChunkHeightmapType::MotionBlocking,
        4,
        4,
        60,
        world.min_y,
    );
    chunk
        .section
        .set_relative_biome(1, 31, 1, pumpkin_data::biome::Biome::PLAINS.id);
    world.set_raining(true);
    // Level.isRaining uses the visual threshold, not the newly set weather flag.
    world.weather.lock().unwrap().rain_level = 1.0;
    let head = BlockPos::new(4, 61, 4);
    world.set_sky_light_level(&head, 15);
    let axolotl = AxolotlEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 60.8, 4.5),
        &EntityType::AXOLOTL,
    ));
    assert!(!world.is_raining_at(&axolotl.get_entity().block_pos.load()));
    assert!(world.is_raining_at(&head));
    axolotl.air_supply.store(1, Relaxed);
    axolotl.after_base_tick();
    assert_eq!(axolotl.air_supply.load(Relaxed), 6000);
    let dolphin = DolphinEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.8, 4.5),
        &EntityType::DOLPHIN,
    ));
    dolphin.moistness_level.store(1, Relaxed);
    dolphin.post_tick();
    assert_eq!(dolphin.moistness_level.load(Relaxed), 2400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strider_lava_fall_exception_does_not_suppress_land_damage() {
    let fixture = crate::entity::death_test_world::DeathTestWorld::new().await;
    let strider = fixture.mob(&EntityType::STRIDER);
    let living = strider.get_living_entity().unwrap();
    let health = living.health.load();
    strider.get_entity().touching_lava.store(true, Relaxed);
    living.fall_distance.store(8.0);
    living.fall(strider.as_ref(), -1.0, true, false);
    assert_eq!(living.health.load(), health);
    assert_eq!(living.fall_distance.load(), 0.0);
    living.fall(strider.as_ref(), -1.0, false, false);
    assert_eq!(living.fall_distance.load(), 0.0);
    strider.get_entity().touching_lava.store(false, Relaxed);
    living.fall_distance.store(8.0);
    living.fall(strider.as_ref(), -1.0, true, false);
    assert!(living.health.load() < health);
}

#[tokio::test]
async fn guardian_moving_with_zero_speed_does_not_sink() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let guardians: [Arc<dyn Mob>; 2] = [
        GuardianEntity::new(Entity::new(
            world.clone(),
            Vector3::new(4.5, 60.0, 4.5),
            &EntityType::GUARDIAN,
        )),
        ElderGuardianEntity::new(Entity::new(
            world,
            Vector3::new(4.5, 60.0, 4.5),
            &EntityType::ELDER_GUARDIAN,
        )),
    ];
    for guardian in guardians {
        let mob = guardian.get_mob_entity();
        mob.living_entity
            .set_attribute_base(&Attributes::MOVEMENT_SPEED, 0.0);
        let target = BlockPos::new(8, 60, 4);
        mob.navigator.lock().unwrap().move_to_path(
            Some(Path::new(vec![Node::new(target)], target, true)),
            1.0,
            &mob.living_entity,
        );
        {
            let mut control = mob.move_control.lock().unwrap();
            control.set_wanted_position(8.5, 60.0, 4.5, 1.0);
            control.tick(guardian.as_ref());
            assert!(control.is_moving());
        };
        let entity = guardian.get_entity();
        entity.touching_water.store(true, Relaxed);
        entity.velocity.store(Vector3::default());
        assert!(guardian.custom_travel(guardian.as_ref()));
        assert_eq!(entity.velocity.load().y, 0.0);
        mob.navigator.lock().unwrap().stop();
        mob.move_control.lock().unwrap().tick(guardian.as_ref());
        entity.velocity.store(Vector3::default());
        assert!(guardian.custom_travel(guardian.as_ref()));
        assert_eq!(entity.velocity.load().y, -0.005);
    }
}

#[tokio::test]
async fn dolphin_initial_air_is_in_spawn_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let dolphin = DolphinEntity::new(Entity::new(world, Vector3::default(), &EntityType::DOLPHIN));
    dolphin.mob_init_data_tracker();
    // Entity metadata index 1, VarInt serializer 1, VarInt(4800), terminator.
    assert_eq!(
        dolphin
            .get_entity()
            .synched_data
            .get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3)
            .unwrap()
            .as_ref(),
        &[1, 1, 0xc0, 0x25, 0xff]
    );
}

#[tokio::test]
async fn spider_fallback_waits_until_the_tick_after_path_completion() {
    use crate::entity::{
        ai::pathfinder::{PathNavigationTrait, WallClimberNavigation},
        mob::MobEntity,
    };
    let directory = tempfile::tempdir().unwrap();
    let world = armor_test_world(directory.path());
    let mob = MobEntity::new(Entity::new(
        world,
        Vector3::new(4.5, 60.0, 4.5),
        &EntityType::SPIDER,
    ));
    mob.living_entity.entity.on_ground.store(true, Relaxed);
    let mut navigation = WallClimberNavigation::new();
    let node = BlockPos::new(4, 60, 4);
    navigation.inner.inner.path = Some(Path::new(vec![Node::new(node)], node, true));
    navigation.path_to_position = Some(BlockPos::new(12, 60, 4));
    navigation.tick(&mob, &mob.living_entity);
    assert!(navigation.inner.is_done());
    assert!(navigation.next_move_target().is_none());
    navigation.tick(&mob, &mob.living_entity);
    assert_eq!(
        navigation.next_move_target().unwrap().0,
        Vector3::new(12.0, 60.0, 4.0)
    );
    // Fallback does not advance the ground navigator's stuck timer.
    assert_eq!(navigation.inner.inner.tick_count, 1);
    navigation.move_to_coords(5.0, 60.0, 4.0, 1.0, &mob.living_entity);
    assert!(navigation.inner.inner.current_goal.is_some());
    navigation.tick(&mob, &mob.living_entity);
    assert!(navigation.inner.inner.current_goal.is_none());
}
