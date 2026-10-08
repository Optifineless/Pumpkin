//! `Zombie.finalizeSpawn` and `handleAttributes`, shared by ordinary and zombified-piglin births.
use crate::entity::{
    EntityBase,
    mob::{
        Mob,
        equipment::RegionalDifficulty,
        spawn::{SpawnGroupData, SpawnReason},
    },
};
use pumpkin_data::entity::EntityType;
use std::sync::Arc;

pub fn finalize_spawn(
    caller: &dyn Mob,
    entity: &Arc<dyn EntityBase>,
    view: &crate::world::spawn_view::SpawnView<'_>,
    difficulty: &RegionalDifficulty,
    reason: super::super::spawn::SpawnReason,
    group_data: Option<SpawnGroupData>,
    set_doors: impl Fn(bool),
) -> SpawnGroupData {
    let world = &entity.get_entity().world.load_full();
    caller.get_mob_entity().finalize_spawn_base();
    let multiplier = difficulty.special_multiplier;
    if reason != SpawnReason::Conversion {
        caller
            .get_mob_entity()
            .set_can_pick_up_loot(rand::random::<f32>() < 0.55 * multiplier);
    }
    let group_data = group_data.unwrap_or_else(|| SpawnGroupData::Zombie {
        is_baby: rand::random::<f32>() < 0.05,
        can_spawn_jockey: true,
    });
    if let SpawnGroupData::Zombie {
        is_baby,
        can_spawn_jockey,
    } = &group_data
    {
        if *is_baby {
            caller.spawn_as_baby();
            if *can_spawn_jockey {
                super::super::spawn::try_chicken_jockey(entity, world, view);
            }
        }
        set_doors(rand::random::<f32>() < multiplier * 0.1);
        if reason != SpawnReason::Conversion {
            super::super::equipment::equip_mob_on_spawn(caller, world, difficulty);
        }
    }
    super::super::equipment::equip_halloween_head(caller.get_mob_entity());
    if handle_attributes(caller, multiplier, reason) {
        set_doors(true);
    }
    group_data
}

fn handle_attributes(
    caller: &dyn Mob,
    multiplier: f32,
    reason: super::super::spawn::SpawnReason,
) -> bool {
    use super::super::spawn::SpawnReason;
    use crate::entity::attributes::{Modifier, ModifierOperation};
    use pumpkin_data::attributes::Attributes;
    // Zombie.handleAttributes (Zombie.java:509); modifiers are permanent except the baby speed.
    let living = &caller.get_mob_entity().living_entity;
    living.set_attribute_base(
        &Attributes::SPAWN_REINFORCEMENTS,
        if caller.get_entity().entity_type == &EntityType::ZOMBIFIED_PIGLIN {
            0.0
        } else {
            rand::random::<f64>() * f64::from(0.1f32)
        },
    );
    let leader;
    {
        let mut attributes = living
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut add = |attribute: &Attributes, id: &str, amount: f64, operation| {
            if let Some(instance) = attributes.get_mut(&attribute.id) {
                instance.add_or_replace_modifier(Modifier {
                    id: id.to_string(),
                    amount,
                    operation,
                    permanent: true,
                });
            }
        };
        add(
            &Attributes::KNOCKBACK_RESISTANCE,
            "minecraft:random_spawn_bonus",
            rand::random::<f64>() * f64::from(0.05f32),
            ModifierOperation::Add,
        );
        let follow_range = rand::random::<f64>() * 1.5 * f64::from(multiplier);
        if follow_range > 1.0 {
            add(
                &Attributes::FOLLOW_RANGE,
                "minecraft:zombie_random_spawn_bonus",
                follow_range,
                ModifierOperation::MultiplyTotal,
            );
        }
        leader = rand::random::<f32>() < multiplier * 0.05;
        if leader {
            add(
                &Attributes::SPAWN_REINFORCEMENTS,
                "minecraft:leader_zombie_bonus",
                rand::random::<f64>() * 0.25 + 0.5,
                ModifierOperation::Add,
            );
            add(
                &Attributes::MAX_HEALTH,
                "minecraft:leader_zombie_bonus",
                rand::random::<f64>() * 3.0 + 1.0,
                ModifierOperation::MultiplyTotal,
            );
        }
    }
    if leader
        && !matches!(
            reason,
            SpawnReason::Conversion | SpawnReason::Load | SpawnReason::DimensionTravel
        )
    {
        living.set_health(living.get_max_health());
    }
    leader
}
