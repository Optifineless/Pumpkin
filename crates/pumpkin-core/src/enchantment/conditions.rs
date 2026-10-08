use super::post_attack::AttackEffectContext;
use crate::entity::EntityBase;
use pumpkin_data::tag::Taggable;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::random::{RandomImpl, xoroshiro128::Xoroshiro};

// Enchantment.damageContext selects the victim, owner, direct source and level.
pub(super) fn matches_requirements(
    requirements: &NbtCompound,
    level: i32,
    victim: &dyn EntityBase,
    context: AttackEffectContext<'_>,
    rng: &mut Xoroshiro,
) -> Option<bool> {
    evaluate_requirements(
        requirements,
        level,
        rng,
        &mut |requirements| match requirements.get_string("type")? {
            "minecraft:entity_properties" => {
                let entity = match requirements.get_string("entity")? {
                    "this" => Some(victim),
                    "attacker" => context.attacker,
                    "direct_attacker" => context.damaging_entity,
                    _ => return None,
                };
                entity.map_or(Some(false), |entity| {
                    requirements
                        .get_compound("predicate")
                        .map_or(Some(true), |predicate| {
                            matches_entity_predicate(predicate, entity)
                        })
                })
            }
            "minecraft:damage_source_properties" => requirements
                .get_compound("predicate")
                .map_or(Some(true), |predicate| {
                    matches_damage_source_predicate(predicate, context)
                }),
            _ => None,
        },
    )
}

// LootItemConditions preserve unsupported predicates through boolean composition.
fn evaluate_requirements(
    requirements: &NbtCompound,
    level: i32,
    rng: &mut Xoroshiro,
    predicate: &mut impl FnMut(&NbtCompound) -> Option<bool>,
) -> Option<bool> {
    if !condition_supported(requirements) {
        return None;
    }
    match requirements.get_string("type")? {
        "minecraft:random_chance" => {
            Some(rng.next_f32() < number_provider(requirements.get("chance")?, level)?)
        }
        "minecraft:all_of" | "minecraft:any_of" => {
            let all = requirements.get_string("type") == Some("minecraft:all_of");
            for term in requirements.get_list("terms")? {
                let matched =
                    evaluate_requirements(term.extract_compound()?, level, rng, predicate)?;
                if matched != all {
                    return Some(matched);
                }
            }
            Some(all)
        }
        "minecraft:inverted" => {
            evaluate_requirements(requirements.get_compound("term")?, level, rng, predicate)
                .map(|matched| !matched)
        }
        _ => predicate(requirements),
    }
}

// Validate support before evaluating, so inversion cannot accept a missing predicate.
fn condition_supported(requirements: &NbtCompound) -> bool {
    match requirements.get_string("type") {
        Some("minecraft:random_chance") => true,
        Some("minecraft:entity_properties") => requirements
            .get_compound("predicate")
            .is_none_or(entity_predicate_supported),
        Some("minecraft:damage_source_properties") => requirements
            .get_compound("predicate")
            .is_none_or(|predicate| {
                predicate
                    .child_tags
                    .iter()
                    .all(|(key, value)| match key.as_ref() {
                        "is_direct" | "tags" => true,
                        "source_entity" | "direct_entity" => value
                            .extract_compound()
                            .is_some_and(entity_predicate_supported),
                        _ => false,
                    })
            }),
        Some("minecraft:all_of" | "minecraft:any_of") => {
            requirements.get_list("terms").is_some_and(|terms| {
                terms
                    .iter()
                    .all(|term| term.extract_compound().is_some_and(condition_supported))
            })
        }
        Some("minecraft:inverted") => requirements
            .get_compound("term")
            .is_some_and(condition_supported),
        _ => false,
    }
}

fn entity_predicate_supported(predicate: &NbtCompound) -> bool {
    predicate
        .child_tags
        .iter()
        .all(|(key, value)| match key.trim_start_matches("minecraft:") {
            "type" | "entity_type" => true,
            "flags" => value.extract_compound().is_some_and(|flags| {
                flags.child_tags.keys().all(|flag| {
                    matches!(
                        flag.as_ref(),
                        "is_flying"
                            | "is_fall_flying"
                            | "is_in_water"
                            | "is_on_ground"
                            | "is_sprinting"
                            | "is_swimming"
                            | "is_on_fire"
                    )
                })
            }),
            "movement" => value.extract_compound().is_some_and(|movement| {
                movement
                    .child_tags
                    .keys()
                    .all(|key| key.as_ref() == "fall_distance")
            }),
            _ => false,
        })
}

// EnchantmentLevelProvider and LevelBasedValue codecs, read from registry NBT.
pub(super) fn number_provider(value: &NbtTag, level: i32) -> Option<f32> {
    if let Some(number) = value.as_numeric_float() {
        return Some(number);
    }
    let compound = value.extract_compound()?;
    match compound.get_string("type")? {
        "minecraft:enchantment_level" => number_provider(compound.get("amount")?, level),
        "minecraft:linear" => Some(
            compound.get_numeric_float("base")?
                + (level - 1) as f32 * compound.get_numeric_float("per_level_above_first")?,
        ),
        "minecraft:levels_squared" => {
            Some((level * level) as f32 + compound.get_numeric_float("added")?)
        }
        "minecraft:clamped" => Some(number_provider(compound.get("value")?, level)?.clamp(
            compound.get_numeric_float("min")?,
            compound.get_numeric_float("max")?,
        )),
        "minecraft:fraction" => {
            let denominator = number_provider(compound.get("denominator")?, level)?;
            Some(if denominator == 0.0 {
                0.0
            } else {
                number_provider(compound.get("numerator")?, level)? / denominator
            })
        }
        "minecraft:lookup" => compound
            .get_list("values")?
            .get((level - 1) as usize)
            .and_then(NbtTag::as_numeric_float)
            .or_else(|| number_provider(compound.get("fallback")?, level)),
        _ => None,
    }
}

fn matches_range(range: &NbtTag, value: f32) -> bool {
    if let Some(exact) = range.as_numeric_float() {
        return value == exact;
    }
    range.extract_compound().is_some_and(|range| {
        range
            .get_numeric_float("min")
            .is_none_or(|min| value >= min)
            && range
                .get_numeric_float("max")
                .is_none_or(|max| value <= max)
    })
}

// EntityPredicate.matches, for the subpredicates used by melee enchantments.
fn matches_entity_predicate(predicate: &NbtCompound, target: &dyn EntityBase) -> Option<bool> {
    if !entity_predicate_supported(predicate) {
        return None;
    }
    let entity = target.get_entity();
    Some(predicate.child_tags.iter().all(
        |(key, value)| match key.trim_start_matches("minecraft:") {
            "type" | "entity_type" => value.extract_string().is_some_and(|kind| {
                kind.strip_prefix('#').map_or_else(
                    || kind.trim_start_matches("minecraft:") == entity.entity_type.registry_key(),
                    |tag| {
                        pumpkin_data::tag::get_tag_ids(
                            pumpkin_data::tag::RegistryKey::EntityType,
                            tag,
                        )
                        .is_some_and(|ids| ids.contains(&entity.entity_type.registry_id()))
                    },
                )
            }),
            "flags" => value.extract_compound().is_some_and(|flags| {
                flags.child_tags.iter().all(|(flag, expected)| {
                    let actual = match flag.as_ref() {
                        "is_flying" => {
                            entity.is_fall_flying()
                                || target
                                    .get_player()
                                    .is_some_and(crate::entity::player::Player::is_flying)
                        }
                        "is_fall_flying" => entity.is_fall_flying(),
                        "is_in_water" => entity.is_in_water(),
                        "is_on_ground" => {
                            entity.on_ground.load(std::sync::atomic::Ordering::Relaxed)
                        }
                        "is_sprinting" => entity.is_sprinting(),
                        "is_swimming" => entity.is_swimming(),
                        "is_on_fire" => {
                            entity.fire_ticks.load(std::sync::atomic::Ordering::Relaxed) > 0
                        }
                        _ => return false,
                    };
                    expected
                        .extract_byte()
                        .is_some_and(|expected| (expected != 0) == actual)
                })
            }),
            "movement" => value.extract_compound().is_some_and(|movement| {
                movement
                    .child_tags
                    .iter()
                    .all(|(component, range)| match component.as_ref() {
                        "fall_distance" => target.get_living_entity().is_some_and(|living| {
                            matches_range(range, living.fall_distance.load())
                        }),
                        _ => false,
                    })
            }),
            _ => false,
        },
    ))
}

// DamageSourcePredicate.matches.
fn matches_damage_source_predicate(
    predicate: &NbtCompound,
    context: AttackEffectContext<'_>,
) -> Option<bool> {
    if predicate.child_tags.iter().any(|(key, _)| {
        !matches!(
            key.as_ref(),
            "is_direct" | "source_entity" | "direct_entity" | "tags"
        )
    }) {
        return None;
    }
    for key in ["source_entity", "direct_entity"] {
        if let Some(predicate) = predicate.get_compound(key) {
            let source = if key == "source_entity" {
                context.attacker
            } else {
                context.damaging_entity
            };
            if let Some(source) = source {
                matches_entity_predicate(predicate, source)?;
            }
        }
    }
    Some(
        predicate
            .child_tags
            .iter()
            .all(|(key, value)| match key.as_ref() {
                "is_direct" => value.extract_byte().is_some_and(|expected| {
                    let direct = context.attacker.map(|owner| owner.get_entity().entity_id)
                        == context
                            .damaging_entity
                            .map(|source| source.get_entity().entity_id);
                    direct == (expected != 0)
                }),
                "source_entity" => context.attacker.is_some_and(|attacker| {
                    value.extract_compound().is_some_and(|predicate| {
                        matches_entity_predicate(predicate, attacker) == Some(true)
                    })
                }),
                "direct_entity" => context.damaging_entity.is_some_and(|source| {
                    value.extract_compound().is_some_and(|predicate| {
                        matches_entity_predicate(predicate, source) == Some(true)
                    })
                }),
                "tags" => value.extract_list().is_some_and(|tags| {
                    tags.iter().all(|tag| {
                        tag.extract_compound().is_some_and(|tag| {
                            tag.get_string("id")
                                .and_then(|id| {
                                    pumpkin_data::tag::get_tag_ids(
                                        pumpkin_data::tag::RegistryKey::DamageType,
                                        id,
                                    )
                                })
                                .is_some_and(|ids| {
                                    ids.contains(&context.damage_type.registry_id())
                                        == tag.get_bool("expected").unwrap_or(true)
                                })
                        })
                    })
                }),
                _ => false,
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn melee_unsupported_condition_remains_unsupported_under_inversion() {
        let mut unsupported = NbtCompound::new();
        unsupported.put_string("type", "test:unknown".into());
        let mut inverted = NbtCompound::new();
        inverted.put_string("type", "minecraft:inverted".into());
        inverted.put("term", NbtTag::Compound(unsupported));
        let mut rng = Xoroshiro::from_seed(1);
        assert_eq!(
            evaluate_requirements(&inverted, 1, &mut rng, &mut |_| None),
            None
        );
    }
}
