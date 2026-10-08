use super::{LivingEntity, damage::HurtContext};
use crate::entity::{
    EntityBase,
    equipment_damage::EquippedItem,
    player::{
        Player,
        statistics::{CustomStatistic, StatisticCategory},
    },
};
use pumpkin_data::{
    Advancement, damage::DamageType, data_component_impl::EquipmentSlot, tag::Taggable,
};
use serde_json::Value;
use std::sync::LazyLock;

// Criterion definitions, including thresholds and source predicates, come from the vanilla pack.
static CRITERIA: LazyLock<Vec<(&'static Advancement, Value)>> = LazyLock::new(|| {
    [
        (Advancement::STORY_DEFLECT_ARROW, include_str!("../../../../../assets/datapack/data/minecraft/advancement/story/deflect_arrow.json")),
        (Advancement::ADVENTURE_SHOOT_ARROW, include_str!("../../../../../assets/datapack/data/minecraft/advancement/adventure/shoot_arrow.json")),
        (Advancement::ADVENTURE_THROW_TRIDENT, include_str!("../../../../../assets/datapack/data/minecraft/advancement/adventure/throw_trident.json")),
        (Advancement::ADVENTURE_OVEROVERKILL, include_str!("../../../../../assets/datapack/data/minecraft/advancement/adventure/overoverkill.json")),
    ].into_iter().filter_map(|(advancement, json)| serde_json::from_str(json).ok().map(|value| (advancement, value))).collect()
});

struct DamagePredicateContext<'a> {
    damage_type: DamageType,
    source: Option<&'a dyn EntityBase>,
    cause: Option<&'a dyn EntityBase>,
    dealt: f32,
    taken: f32,
    blocked: bool,
}

impl LivingEntity {
    // LivingEntity.hurtServer 1279-1288: trigger even when a fully blocked admitted hit returns false.
    pub(super) fn damage_criteria_and_block_stat(
        caller: &dyn EntityBase,
        hurt: HurtContext<'_>,
        dealt: f32,
        taken: f32,
        blocked: f32,
    ) {
        let context = DamagePredicateContext {
            damage_type: hurt.damage_type,
            source: hurt.source,
            cause: hurt.cause,
            dealt,
            taken,
            blocked: blocked > 0.0,
        };
        if let Some(player) = caller.get_player() {
            trigger(player, "minecraft:entity_hurt_player", &context);
            if blocked > 0.0 && blocked < f32::MAX / 10.0 {
                player.increment_stat(
                    StatisticCategory::Custom,
                    CustomStatistic::DamageBlockedByShield as i32,
                    (blocked * 10.0).round() as i32,
                );
            }
        }
        if let Some(player) = hurt.cause.and_then(EntityBase::get_player) {
            trigger(player, "minecraft:player_hurt_entity", &context);
        }
    }
}

// EntityHurtPlayerTrigger.trigger / PlayerHurtEntityTrigger.trigger / DamagePredicate.matches.
fn trigger(player: &Player, name: &str, context: &DamagePredicateContext<'_>) {
    for (advancement, definition) in CRITERIA.iter() {
        if let Some(criteria) = definition.get("criteria").and_then(Value::as_object) {
            for (id, criterion) in criteria {
                if criterion.get("trigger").and_then(Value::as_str) == Some(name)
                    && criterion
                        .get("conditions")
                        .and_then(|conditions| conditions.get("damage"))
                        .is_none_or(|predicate| matches_damage(predicate, context))
                {
                    player.trigger_advancement_criterion(advancement, id);
                }
            }
        }
    }
}

fn matches_damage(predicate: &Value, context: &DamagePredicateContext<'_>) -> bool {
    predicate.as_object().is_some_and(|fields| {
        fields.iter().all(|(name, value)| match name.as_str() {
            "dealt" => matches_range(value, context.dealt),
            "taken" => matches_range(value, context.taken),
            "blocked" => value.as_bool() == Some(context.blocked),
            "type" => value.as_object().is_some_and(|fields| {
                fields.iter().all(|(name, value)| match name.as_str() {
                    "direct_entity" => context
                        .source
                        .is_some_and(|source| matches_entity(value, source)),
                    "source_entity" => context
                        .cause
                        .is_some_and(|cause| matches_entity(value, cause)),
                    "tags" => value.as_array().is_some_and(|tags| {
                        tags.iter().all(|entry| {
                            entry.get("id").and_then(Value::as_str).is_some_and(|id| {
                                context
                                    .damage_type
                                    .is_tagged_with(id.trim_start_matches('#'))
                                    == entry.get("expected").and_then(Value::as_bool)
                            })
                        })
                    }),
                    _ => false,
                })
            }),
            _ => false,
        })
    })
}

fn matches_range(range: &Value, value: f32) -> bool {
    if let Some(exact) = range.as_f64() {
        return f64::from(value) == exact;
    }
    range.as_object().is_some_and(|bounds| {
        bounds.iter().all(|(key, bound)| {
            bound.as_f64().is_some_and(|bound| match key.as_str() {
                "min" => f64::from(value) >= bound,
                "max" => f64::from(value) <= bound,
                _ => false,
            })
        })
    })
}

fn matches_entity(predicate: &Value, entity: &dyn EntityBase) -> bool {
    predicate.as_object().is_some_and(|fields| {
        fields
            .iter()
            .all(|(key, value)| match key.trim_start_matches("minecraft:") {
                "entity_type" => value.as_str().is_some_and(|kind| {
                    kind.strip_prefix('#').map_or_else(
                        || {
                            kind.trim_start_matches("minecraft:")
                                == entity.get_entity().entity_type.resource_name
                        },
                        |tag| entity.get_entity().entity_type.is_tagged_with(tag) == Some(true),
                    )
                }),
                "equipment" => value.as_object().is_some_and(|equipment| {
                    equipment.iter().all(|(slot, predicate)| {
                        if slot != "mainhand" {
                            return false;
                        }
                        let item = EquippedItem::capture(entity, &EquipmentSlot::MAIN_HAND);
                        predicate
                            .get("items")
                            .and_then(Value::as_str)
                            .is_some_and(|name| {
                                name.trim_start_matches("minecraft:")
                                    == item.stack.item.registry_key
                            })
                    })
                }),
                _ => false,
            })
    })
}
