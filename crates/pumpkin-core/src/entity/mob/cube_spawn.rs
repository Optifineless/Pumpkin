//! `AbstractCubeMob.readAdditionalSaveData` at configured entity creation boundaries.

use super::{
    Mob,
    equipment::RegionalDifficulty,
    spawn::{AgeableGroupData, SpawnGroupData},
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::random::{RandomGenerator, RandomImpl};

/// `AbstractCubeMob` reads numeric Size and performs Java's wrapping addition before `setSize`.
pub(super) fn read_size(nbt: &NbtCompound) -> i32 {
    numeric_size(nbt.get("Size")).wrapping_add(1)
}

/// `Slime`/`MagmaCube.finalizeSpawn` -> `AgeableMob` -> `Mob` -> `AbstractCubeMob.setSpawnSize`.
pub(super) fn finalize_spawn(
    mob: &dyn Mob,
    difficulty: &RegionalDifficulty,
    group: Option<SpawnGroupData>,
    random: &mut RandomGenerator,
    set_size: impl FnOnce(i32, bool),
) -> SpawnGroupData {
    // Both concrete finalizers supply AgeableMobGroupData(false), preserving a supplied group.
    let mut data = match group {
        Some(SpawnGroupData::Ageable(data)) => data,
        _ => AgeableGroupData::new(false, 0.05),
    };
    if data.should_spawn_baby && data.group_size > 0 {
        // canBeABaby is false for both cube species; AgeableMob still consumes this roll.
        random.next_f32();
    }
    data.group_size += 1;
    mob.get_mob_entity().finalize_spawn_base();
    let mut scale = random.next_bounded_i32(3);
    if scale < 2 && random.next_f32() < 0.5 * difficulty.special_multiplier {
        scale += 1;
    }
    set_size(1 << scale, true);
    SpawnGroupData::Ageable(data)
}

fn numeric_size(tag: Option<&NbtTag>) -> i32 {
    match tag {
        Some(NbtTag::Byte(value)) => i32::from(*value),
        Some(NbtTag::Short(value)) => i32::from(*value),
        Some(NbtTag::Int(value)) => *value,
        Some(NbtTag::Long(value)) => *value as i32,
        Some(NbtTag::Float(value)) => value.floor() as i32,
        Some(NbtTag::Double(value)) => value.floor() as i32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::{
            mob::{magma_cube::MagmaCubeEntity, slime::SlimeEntity},
            r#type::from_type,
        },
        world::spawn_test_support::Fixture,
    };
    use pumpkin_data::{attributes::Attributes, entity::EntityType};
    use pumpkin_util::{Difficulty, math::vector3::Vector3, random::legacy_rand::LegacyRand};

    #[tokio::test]
    async fn cube_load_size_precedes_living_health_and_attributes() {
        let fixture = Fixture::new();
        for ty in [&EntityType::SLIME, &EntityType::MAGMA_CUBE] {
            for (tag, size) in [
                (NbtTag::Byte(3), 4),
                (NbtTag::Short(1), 2),
                (NbtTag::Double(3.9), 4),
                (NbtTag::Int(i32::MAX), 1),
            ] {
                let mut nbt = NbtCompound::new();
                nbt.put_string("id", format!("minecraft:{}", ty.resource_name));
                nbt.put("Size", tag);
                nbt.put_bool("wasOnGround", true);
                let entity = super::super::spawn::load_spawn_entity(
                    &fixture.world,
                    &nbt,
                    Vector3::new(8.5, 64.0, 8.5),
                )
                .unwrap();
                let living = entity.get_living_entity().unwrap();
                assert_eq!(living.health.load(), (size * size) as f32);
                assert_eq!(
                    living.get_attribute_value(&Attributes::ATTACK_DAMAGE),
                    f64::from(size)
                );
                let mut saved = NbtCompound::new();
                entity.write_nbt(&mut saved);
                assert_eq!(saved.get_int("Size"), Some(size - 1));
                assert_eq!(saved.get_bool("wasOnGround"), Some(true));
                nbt.put_float("Health", 0.5);
                entity.read_nbt_non_mut(&nbt);
                assert_eq!(living.health.load(), 0.5);
                living.set_attribute_base(&Attributes::MAX_HEALTH, 20.0);
                entity.write_nbt(&mut saved);
                entity.read_nbt_non_mut(&saved);
                assert_eq!(living.get_max_health(), 20.0);
                assert_eq!(living.health.load(), 0.5);
            }
        }
        fixture.finish().await;
    }

    #[tokio::test]
    async fn cube_finalizers_roll_size_and_reset_health() {
        let fixture = Fixture::new();
        for ty in [&EntityType::SLIME, &EntityType::MAGMA_CUBE] {
            let entity = from_type(
                ty,
                Vector3::new(8.5, 64.0, 8.5),
                &fixture.world,
                uuid::Uuid::new_v4(),
            );
            let mob = entity.get_mob().unwrap();
            for (difficulty, expected) in [(Difficulty::Easy, 2), (Difficulty::Hard, 4)] {
                let difficulty =
                    RegionalDifficulty::calculate(difficulty, 3_600_000, 3_600_000, 1.0);
                let group = finalize_spawn(
                    mob,
                    &difficulty,
                    None,
                    &mut RandomGenerator::Legacy(LegacyRand::from_seed(2)),
                    |size, reset| {
                        if let Some(slime) = entity.cast_any().downcast_ref::<SlimeEntity>() {
                            slime.set_size(size, reset);
                        } else {
                            entity
                                .cast_any()
                                .downcast_ref::<MagmaCubeEntity>()
                                .unwrap()
                                .set_size(size, reset);
                        }
                    },
                );
                assert!(matches!(
                    group,
                    SpawnGroupData::Ageable(AgeableGroupData {
                        group_size: 1,
                        should_spawn_baby: false,
                        ..
                    })
                ));
                let living = entity.get_living_entity().unwrap();
                assert_eq!(living.health.load(), (expected * expected) as f32);
                if ty == &EntityType::MAGMA_CUBE {
                    assert_eq!(
                        living.get_attribute_value(&Attributes::ARMOR),
                        f64::from(expected * 3)
                    );
                }
            }
            // Exercise subtype dispatch and health reset after the deterministic size checks.
            entity.get_living_entity().unwrap().health.store(0.5);
            let group = super::super::spawn::finalize_spawn_with_reason(
                &entity,
                &fixture.world,
                super::super::spawn::SpawnReason::TrialSpawner,
                None,
            );
            assert!(matches!(group, Some(SpawnGroupData::Ageable(_))));
            let living = entity.get_living_entity().unwrap();
            assert_eq!(living.health.load(), living.get_max_health());
        }
        fixture.finish().await;
    }
}
