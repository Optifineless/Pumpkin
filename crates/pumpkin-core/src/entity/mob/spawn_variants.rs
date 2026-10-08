//! The `SpawnContext`-consuming `finalizeSpawn` overrides and `VariantUtils`/`PriorityProvider.pick`.

use std::sync::Arc;

use pumpkin_data::spawn_variant::{Condition, selectors};
use rand::seq::IndexedRandom;

use super::spawn::SpawnGroupData;
use crate::{entity::EntityBase, world::spawn_view::SpawnView};

/// Inherits a cow, pig or chicken parent's variant without fresh-spawn finalization.
/// Both parents and the offspring must have the same species; other species are unchanged.
pub fn inherit_breeding_variant(
    parent: &dyn EntityBase,
    partner: &dyn EntityBase,
    offspring: &dyn EntityBase,
) {
    use crate::entity::passive::{chicken::ChickenEntity, cow::CowEntity, pig::PigEntity};
    use pumpkin_data::{
        chicken_variant::ChickenVariant, cow_variant::CowVariant, entity::EntityType,
        pig_variant::PigVariant,
    };
    use std::sync::atomic::Ordering::Relaxed;

    let ty = parent.get_entity().entity_type;
    if ty != partner.get_entity().entity_type
        || ty != offspring.get_entity().entity_type
        || ![&EntityType::COW, &EntityType::PIG, &EntityType::CHICKEN].contains(&ty)
    {
        return;
    }
    // Cow/Pig/Chicken.getBreedOffspring chooses one parent's variant with nextBoolean.
    let source = if rand::random() { parent } else { partner };
    let name = if let Some(cow) = source.cast_any().downcast_ref::<CowEntity>() {
        CowVariant::from_id(cow.variant.load(Relaxed))
            .unwrap_or_default()
            .to_name()
    } else if let Some(pig) = source.cast_any().downcast_ref::<PigEntity>() {
        PigVariant::from_id(pig.variant.load(Relaxed))
            .unwrap_or_default()
            .to_name()
    } else if let Some(chicken) = source.cast_any().downcast_ref::<ChickenEntity>() {
        ChickenVariant::from_id(chicken.variant.load(Relaxed))
            .unwrap_or_default()
            .to_name()
    } else {
        return;
    };
    offspring.set_variant_name(name);
}

pub(super) fn finalize_variants(
    entity: &Arc<dyn EntityBase>,
    view: &SpawnView<'_>,
    group: Option<SpawnGroupData>,
) -> Option<SpawnGroupData> {
    let base = entity.get_entity();
    let species = base.entity_type.resource_name;
    let selectors = selectors(species);
    if selectors.is_empty() {
        return group;
    }
    let pos = base.block_pos.load();
    let biome = view.get_biome(&pos);
    let mut highest = i32::MIN;
    let mut candidates = Vec::new();
    for selector in selectors {
        if selector.priority < highest {
            continue;
        }
        let matches = match selector.condition {
            Condition::Always => true,
            Condition::Biomes(names) => names
                .iter()
                .any(|name| name.strip_prefix("minecraft:").unwrap_or(name) == biome.registry_id),
            Condition::Structures(names) => view.has_structure_piece(&pos, names),
            Condition::MoonBrightness { min, max } => {
                let brightness = f64::from(view.moon_brightness());
                (min..=max).contains(&brightness)
            }
        };
        if matches {
            if selector.priority > highest {
                candidates.clear();
                highest = selector.priority;
            }
            candidates.push(selector.name);
        }
    }
    // Wolf.finalizeSpawn reuses the first wolf's variant and disables babies for the pack.
    let selected = if let Some(SpawnGroupData::Wolf { variant, .. }) = &group {
        Some(*variant)
    } else {
        candidates.choose(&mut rand::rng()).copied()
    };
    // Each of these finalizeSpawn overrides also selects from its sound-variant registry.
    if let Some(mob) = entity.get_mob() {
        let mut random = rand::rng();
        let sound = match species {
            "cow" => pumpkin_data::cow_sound_variant::CowSoundVariant::all()
                .choose(&mut random)
                .map(pumpkin_data::cow_sound_variant::CowSoundVariant::to_name),
            "pig" => pumpkin_data::pig_sound_variant::PigSoundVariant::all()
                .choose(&mut random)
                .map(pumpkin_data::pig_sound_variant::PigSoundVariant::to_name),
            "chicken" => pumpkin_data::chicken_sound_variant::ChickenSoundVariant::all()
                .choose(&mut random)
                .map(pumpkin_data::chicken_sound_variant::ChickenSoundVariant::to_name),
            "cat" => pumpkin_data::cat_sound_variant::CatSoundVariant::all()
                .choose(&mut random)
                .map(pumpkin_data::cat_sound_variant::CatSoundVariant::to_name),
            "wolf" => pumpkin_data::wolf_sound_variant::WolfSoundVariant::all()
                .choose(&mut random)
                .map(pumpkin_data::wolf_sound_variant::WolfSoundVariant::to_name),
            _ => None,
        };
        if let Some(sound) = sound {
            mob.mob_set_sound_variant_name(sound);
        }
    }
    if let Some(name) = selected {
        if let Some(mob) = entity.get_mob() {
            mob.mob_set_variant_name(name);
        }
        if species == "wolf" && !matches!(group, Some(SpawnGroupData::Wolf { .. })) {
            return Some(SpawnGroupData::Wolf {
                variant: name,
                ageable: super::spawn::ageable_group_data(base.entity_type),
            });
        }
    }
    group
}
