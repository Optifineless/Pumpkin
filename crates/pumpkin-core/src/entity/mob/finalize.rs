//! Fresh mob finalization, separated from restorative metadata initialization.

use std::sync::Arc;

use super::{Mob, MobEntity, equipment, spawn};
use crate::world::World;

// Mob.finalizeSpawn and AgeableMob.finalizeSpawn; species hooks keep their superclass order.
pub(super) fn finalize_spawn<M: Mob + ?Sized>(
    mob: &M,
    world: &Arc<World>,
    view: &crate::world::spawn_view::SpawnView<'_>,
    group_data: Option<spawn::SpawnGroupData>,
) -> Option<spawn::SpawnGroupData> {
    // AgeableMob.finalizeSpawn selects age before calling Mob.finalizeSpawn.
    let group_data = if let Some(ageable) = mob.as_ageable() {
        let wolf_variant = match &group_data {
            Some(spawn::SpawnGroupData::Wolf { variant, .. }) => Some(*variant),
            _ => None,
        };
        let mut data = match group_data {
            Some(
                spawn::SpawnGroupData::Ageable(data)
                | spawn::SpawnGroupData::Wolf { ageable: data, .. },
            ) => data,
            _ => spawn::ageable_group_data(mob.get_entity().entity_type),
        };
        let roll = if data.should_spawn_baby && data.group_size >= data.adult_count {
            rand::random()
        } else {
            1.0
        };
        if data.next_is_baby(roll) {
            ageable.set_age(ageable.get_baby_start_age());
        }
        Some(match wolf_variant {
            Some(variant) => spawn::SpawnGroupData::Wolf {
                variant,
                ageable: data,
            },
            None => spawn::SpawnGroupData::Ageable(data),
        })
    } else {
        group_data
    };
    mob.get_mob_entity().finalize_spawn_base();
    let difficulty = view.difficulty_at(mob.get_entity().pos.load());
    equipment::equip_mob_on_spawn(mob, world, &difficulty);
    if equipment::EQUIPMENT_REGISTRY
        .get(mob.get_entity().entity_type.resource_name)
        .is_some_and(|definition| definition.can_pick_up_loot)
    {
        mob.get_mob_entity().set_can_pick_up_loot(
            rand::random::<f32>()
                < MobEntity::MAX_PICKUP_LOOT_CHANCE * difficulty.special_multiplier,
        );
    }
    group_data
}
