use super::{
    LootCondition, LootContextParameters, LootRandom, MAX_LOOT_DEPTH, attacker_enchantment_level,
    enchantment_level, entity_predicate, entity_target, item_predicate, match_block, number_float,
    registry_key,
};
use serde_json::Value;
use std::collections::BTreeSet;
fn condition_result(
    condition: &LootCondition,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
    depth: usize,
) -> Option<bool> {
    if depth >= MAX_LOOT_DEPTH || !rng.charge_work(1) {
        return None;
    }
    Some(match condition {
        LootCondition::None => true,
        LootCondition::Unsupported(_) => return None,
        LootCondition::Reference(name) => condition_result(
            super::registry::predicate(params, name)?.as_ref(),
            params,
            rng,
            depth + 1,
        )?,
        LootCondition::SurvivesExplosion => params
            .explosion_radius
            .is_none_or(|radius| rng.next_f32() <= 1.0 / radius),
        LootCondition::KilledByPlayer => {
            // LootItemKilledByPlayerCondition.test checks LAST_DAMAGE_PLAYER, not kill credit.
            params.last_damage_player_state.is_some()
        }
        LootCondition::RandomChance(chance) => {
            let probability = number_float(chance, rng, 0)?;
            rng.next_f32() < probability
        }
        LootCondition::RandomChanceWithEnchantedBonus {
            enchantment,
            unenchanted,
            enchanted,
        } => {
            let level = attacker_enchantment_level(params, enchantment);
            let chance = if level == 0 {
                *unenchanted
            } else if let Some(chance) = enchanted.as_f64() {
                chance as f32
            } else {
                enchanted.get("base").and_then(Value::as_f64).unwrap_or(0.0) as f32
                    + enchanted
                        .get("per_level_above_first")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0) as f32
                        * (level - 1) as f32
            };
            rng.next_f32() < chance
        }
        LootCondition::TableBonus {
            enchantment,
            chances,
        } => {
            let level = enchantment_level(params.tool.as_ref(), enchantment) as usize;
            chances
                .get(level.min(chances.len().saturating_sub(1)))
                .is_some_and(|chance| rng.next_f32() < *chance)
        }
        LootCondition::BlockStateProperty { blocks, properties } => {
            match_block(blocks, properties, params)
        }
        LootCondition::MatchTool(predicate) => match params.tool.as_ref() {
            Some(tool) => item_predicate(predicate, tool)?,
            None => false,
        },
        LootCondition::EntityProperties { entity, predicate } => {
            // LootItemEntityPropertyCondition.test treats an omitted predicate as unconditional.
            predicate.is_null()
                || match entity_target(entity, params) {
                    Some(entity) => entity_predicate(predicate, entity)?,
                    None => false,
                }
        }
        LootCondition::Inverted(term) => !condition_result(term, params, rng, depth + 1)?,
        LootCondition::AllOf(terms) => {
            for term in terms {
                if !condition_result(term, params, rng, depth + 1)? {
                    return Some(false);
                }
            }
            true
        }
        LootCondition::AnyOf(terms) => {
            let mut matched = false;
            for term in terms {
                if condition_result(term, params, rng, depth + 1)? {
                    matched = true;
                    break;
                }
            }
            matched
        }
        LootCondition::WeatherCheck {
            raining,
            thundering,
        } => {
            raining.is_none_or(|r| params.is_raining == Some(r))
                && thundering.is_none_or(|t| params.is_thundering == Some(t))
        }
    })
}
pub(super) fn check_condition(
    condition: &LootCondition,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
) -> bool {
    // Validate the complete holder graph before inversion, but evaluate with vanilla short circuiting.
    let mut references = ConditionReferences {
        params,
        visiting: BTreeSet::new(),
        supported: BTreeSet::new(),
    };
    references.validate(condition, rng, 0)
        && condition_result(condition, params, rng, 0).unwrap_or(false)
}
struct ConditionReferences<'a> {
    params: &'a LootContextParameters,
    visiting: BTreeSet<String>,
    supported: BTreeSet<String>,
}
impl ConditionReferences<'_> {
    fn validate(
        &mut self,
        condition: &LootCondition,
        rng: &mut LootRandom<'_>,
        depth: usize,
    ) -> bool {
        if depth >= MAX_LOOT_DEPTH || !rng.charge_work(1) {
            return false;
        }
        match condition {
            LootCondition::Unsupported(_) => false,
            LootCondition::Reference(name) => {
                let name = registry_key(name);
                if self.supported.contains(&name) {
                    return true;
                }
                if !self.visiting.insert(name.clone()) {
                    return false;
                }
                let supported = super::registry::predicate(self.params, &name)
                    .is_some_and(|value| self.validate(&value, rng, depth + 1));
                self.visiting.remove(&name);
                if supported {
                    self.supported.insert(name);
                }
                supported
            }
            LootCondition::Inverted(term) => self.validate(term, rng, depth + 1),
            LootCondition::AllOf(terms) | LootCondition::AnyOf(terms) => {
                terms.iter().all(|term| self.validate(term, rng, depth + 1))
            }
            _ => true,
        }
    }
}
