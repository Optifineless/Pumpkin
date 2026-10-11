use super::{
    generation_spawning,
    spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};

#[tokio::test]
async fn generation_births_use_region_at_night_through_publication() {
    check_generation_births(&[
        (&EntityType::RABBIT, &Biome::DESERT, &Block::SAND, None),
        (&EntityType::GOAT, &Biome::JAGGED_PEAKS, &Block::STONE, None),
    ])
    .await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn generation_variants_are_selected_before_retention_and_restore() {
    check_generation_births(&[
        (
            &EntityType::COW,
            &Biome::SAVANNA,
            &Block::GRASS_BLOCK,
            Some("minecraft:warm"),
        ),
        (
            &EntityType::PIG,
            &Biome::SAVANNA,
            &Block::GRASS_BLOCK,
            Some("minecraft:warm"),
        ),
        (
            &EntityType::CHICKEN,
            &Biome::SAVANNA,
            &Block::GRASS_BLOCK,
            Some("minecraft:warm"),
        ),
        (
            &EntityType::WOLF,
            &Biome::SNOWY_TAIGA,
            &Block::GRASS_BLOCK,
            Some("minecraft:ashen"),
        ),
        (
            &EntityType::FROG,
            &Biome::MANGROVE_SWAMP,
            &Block::GRASS_BLOCK,
            Some("minecraft:warm"),
        ),
    ])
    .await;
    crate::server::fixture_lifecycle::finish().await;
}

async fn check_generation_births(
    cases: &[(
        &'static EntityType,
        &'static Biome,
        &'static Block,
        Option<&str>,
    )],
) {
    let fixture = Fixture::new();
    let world = &fixture.world;
    world.level_time.lock().unwrap().time_of_day = 18000;
    assert!(world.get_sky_darken() > 0);
    for &(ty, biome, floor, variant) in cases {
        world.level.loaded_chunks.clear();
        world.entities.store(std::sync::Arc::new(Vec::new()));
        let mut chunk = proto(biome, floor);
        let pos = BlockPos::new(8, 64, 8);
        assert_eq!(world.get_biome(&pos), &Biome::PLAINS);
        assert!(
            generation_spawning::spawn_mob(
                world,
                &mut chunk,
                ty,
                Vector3::new(8.5, 64.0, 8.5),
                &mut None
            ),
            "{}",
            ty.resource_name
        );
        assert!(world.entities.load().is_empty());
        if let Some(variant) = variant {
            assert_eq!(
                chunk.pending_entities[0].get_string("variant"),
                Some(variant)
            );
        }
        let id = chunk.pending_entities[0].get_uuid("UUID").unwrap();
        publish(world, chunk);
        world.publish_generated_entities(Vector2::new(0, 0));
        let restored = world.get_entity_by_uuid(id).unwrap();
        let mut saved = pumpkin_nbt::compound::NbtCompound::new();
        restored.write_nbt(&mut saved);
        if let Some(variant) = variant {
            assert_eq!(saved.get_string("variant"), Some(variant));
        }
    }
    fixture.finish().await;
}

#[tokio::test]
async fn trial_spawner_rejects_custom_light_and_applies_stock_equipment() {
    use crate::block::entities::trial_spawner::{TrialSpawner, TrialSpawnerConfig};
    use pumpkin_data::{data_component_impl::EquipmentSlot, item::Item};
    use pumpkin_nbt::compound::NbtCompound;
    let fixture = Fixture::new();
    let world = &fixture.world;
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
    // Keep all three random spawn heights clear of the floor for the sight ray.
    let spawner_pos = BlockPos::new(8, 66, 8);
    let mut trial = TrialSpawner {
        is_ominous: true,
        ..TrialSpawner::default()
    };
    trial.config.ominous = TrialSpawnerConfig::from(
        &pumpkin_data::trial_spawner::TRIAL_CHAMBER_RANGED_SKELETON_OMINOUS,
    );
    trial.config.ominous.spawn_range = 0;
    let mut data = trial.config.ominous.spawn_potentials[0].data.clone();
    let mut rules = NbtCompound::new();
    let mut range = NbtCompound::new();
    range.put_int("min_inclusive", 1);
    range.put_int("max_inclusive", 15);
    rules.put_compound("block_light_limit", range);
    data.custom_spawn_rules = Some(rules);
    trial.data.next_spawn_data = Some(data.clone());
    assert!(trial.spawn_mob(world, spawner_pos).is_none());
    assert!(world.entities.load().is_empty());
    data.custom_spawn_rules = None;
    trial.data.next_spawn_data = Some(data);
    let id = trial.spawn_mob(world, spawner_pos).unwrap();
    let skeleton = world.get_entity_by_uuid(id).unwrap();
    let living = skeleton.get_living_entity().unwrap();
    assert_eq!(
        living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND)
            .item,
        &Item::BOW
    );
    assert_eq!(
        living
            .equipment_drop_chances
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND),
        Some(&0.0)
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn structure_generation_feeds_runtime_spawn_overrides() {
    use pumpkin_data::{dimension::Dimension, entity::MobCategory, structures::StructureKeys};
    use pumpkin_util::world_seed::Seed;
    use pumpkin_world::{
        chunk_system::chunk_state::StagedChunkEnum,
        generation::{
            generator::WorldGenerator, get_world_gen, proto_chunk::ProtoChunk,
            spawn_structures::structure_bounds_from_data,
        },
    };
    let fixture = Fixture::new();
    // Same vanilla monument edge fixture as pumpkin-world's chunk resume regression.
    let generator = get_world_gen(
        Seed(1_782_124_772_053_846_960),
        Dimension::OVERWORLD,
        false,
        Vec::new(),
        String::new(),
    );
    let WorldGenerator::Noise(noise) = &*generator else {
        panic!("fixture generator must use noise")
    };
    let mut chunk = ProtoChunk::new(-553, 174, &generator);
    chunk.step_to_biomes(noise);
    chunk.set_structure_starts(noise);
    chunk.set_structure_references(noise);
    let bound = chunk
        .spawn_structures
        .iter()
        .find(|b| b.structure == StructureKeys::Monument)
        .unwrap();
    let pos = BlockPos::new(
        (-553 * 16 + 8).clamp(bound.full.min.x, bound.full.max.x),
        bound.full.min.y,
        (174 * 16 + 8).clamp(bound.full.min.z, bound.full.max.z),
    );
    // Completed chunks must retain references without rebuilding structure starts on reload.
    chunk.stage = StagedChunkEnum::Spawn;
    let chunk = publish(&fixture.world, chunk);
    assert!(
        !structure_bounds_from_data(
            chunk
                .get_custom_data("murgicraft", "spawn_structures")
                .as_ref()
        )
        .is_empty()
    );
    let spawns = super::natural_spawner::mobs_at(&fixture.world, &MobCategory::MONSTER, &pos);
    assert_eq!(spawns.len(), 1);
    assert_eq!(spawns[0].r#type, "minecraft:guardian");
    let resumed = ProtoChunk::from_chunk_data(&chunk, &generator);
    assert!(
        resumed
            .spawn_structures
            .iter()
            .any(|b| b.structure == StructureKeys::Monument)
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn species_persistence_and_chicken_jockey_state_survive_entity_loading() {
    use crate::entity::{mob::spawn::load_spawn_entity, r#type::from_type};
    use pumpkin_nbt::compound::NbtCompound;
    let fixture = Fixture::new();
    for ty in [
        &EntityType::VILLAGER,
        &EntityType::COW,
        &EntityType::IRON_GOLEM,
        &EntityType::PARROT,
    ] {
        let entity = from_type(
            ty,
            Vector3::new(8.5, 64.0, 8.5),
            &fixture.world,
            uuid::Uuid::new_v4(),
        );
        assert!(!entity.get_mob().unwrap().remove_when_far_away(20000.0));
    }
    for ty in [&EntityType::COD, &EntityType::AXOLOTL, &EntityType::TADPOLE] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("id", format!("minecraft:{}", ty.resource_name));
        nbt.put_bool("FromBucket", true);
        let entity = load_spawn_entity(&fixture.world, &nbt, Vector3::new(8.5, 64.0, 8.5)).unwrap();
        assert!(entity.get_mob().unwrap().requires_custom_persistence());
        assert!(!entity.get_mob().unwrap().remove_when_far_away(20000.0));
    }
    let chicken = from_type(
        &EntityType::CHICKEN,
        Vector3::new(8.5, 64.0, 8.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    let mob = chicken.get_mob().unwrap();
    mob.set_chicken_jockey(true);
    assert!(mob.remove_when_far_away(20000.0));
    let mut nbt = NbtCompound::new();
    chicken.write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("IsChickenJockey"), Some(true));
    nbt.put_int("EggLayTime", 1);
    let restored = load_spawn_entity(&fixture.world, &nbt, Vector3::new(8.5, 64.0, 8.5)).unwrap();
    restored.get_mob().unwrap().mob_tick(restored.as_ref());
    restored.write_nbt(&mut nbt);
    assert_eq!(nbt.get_int("EggLayTime"), Some(1));
    assert!(restored.get_mob().unwrap().remove_when_far_away(20000.0));
    assert!(fixture.world.entities.load().is_empty());
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn zombified_piglin_inherits_baby_group_and_zero_reinforcement_base() {
    use crate::entity::{
        mob::spawn::{SpawnGroupData, SpawnReason, finalize_spawn_with_reason},
        r#type::from_type,
    };
    use pumpkin_data::attributes::Attributes;
    use pumpkin_nbt::compound::NbtCompound;
    let fixture = Fixture::new();
    fixture.world.set_difficulty(pumpkin_util::Difficulty::Easy);
    let entity = from_type(
        &EntityType::ZOMBIFIED_PIGLIN,
        Vector3::new(8.5, 64.0, 8.5),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    finalize_spawn_with_reason(
        &entity,
        &fixture.world,
        SpawnReason::Natural,
        Some(SpawnGroupData::Zombie {
            is_baby: true,
            can_spawn_jockey: false,
        }),
    );
    let mut nbt = NbtCompound::new();
    entity.write_nbt(&mut nbt);
    assert_eq!(nbt.get_bool("IsBaby"), Some(true));
    let living = entity.get_living_entity().unwrap();
    assert_eq!(
        living.get_attribute_value(&Attributes::SPAWN_REINFORCEMENTS),
        0.0
    );
    assert_eq!(entity.get_entity().entity_dimension.load().eye_height, 0.78);
    let speed = living.get_attribute_value(&Attributes::MOVEMENT_SPEED);
    entity.read_nbt_non_mut(&nbt);
    assert_eq!(
        living.get_attribute_value(&Attributes::MOVEMENT_SPEED),
        speed
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn cat_finalize_uses_unpublished_structure_context() {
    use pumpkin_data::structures::StructureKeys;
    use pumpkin_util::math::block_box::BlockBox;
    use pumpkin_world::generation::spawn_structures::StructureSpawnBounds;
    let fixture = Fixture::new();
    let mut chunk = proto(&Biome::SWAMP, &Block::GRASS_BLOCK);
    let bounds = BlockBox::new(0, 63, 0, 15, 70, 15);
    chunk.spawn_structures.push(StructureSpawnBounds {
        structure: StructureKeys::SwampHut,
        full: bounds,
        pieces: vec![bounds],
    });
    assert!(generation_spawning::spawn_mob(
        &fixture.world,
        &mut chunk,
        &EntityType::CAT,
        Vector3::new(8.5, 64.0, 8.5),
        &mut None
    ));
    assert_eq!(
        chunk.pending_entities[0].get_string("variant"),
        Some("minecraft:all_black")
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}
