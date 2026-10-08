use super::{
    LootContextParameters, LootFunction, LootRandom, MAX_LOOT_DEPTH, MAX_LOOT_ROLLS,
    attacker_enchantment_level, check_condition, enchantment_level, entity_target, item_component,
    number_float, number_int, read_loot_component, registry_key,
};
use pumpkin_data::{data_component::DataComponent, item::Item, item_stack::ItemStack};
use serde_json::Value;
use std::collections::BTreeSet;
pub(super) struct LootStack {
    pub(super) stack: ItemStack,
    pub(super) count: i32,
}
impl LootStack {
    // ItemStack.getCount reads zero for AIR and nonpositive stored counts.
    fn get_count(&self) -> i32 {
        if self.stack.item == &Item::AIR {
            0
        } else {
            self.count.max(0)
        }
    }
}
pub(super) fn new_stack(name: &str) -> Option<LootStack> {
    Item::from_registry_key(name.trim_start_matches("minecraft:")).map(|item| LootStack {
        stack: ItemStack::new(1, item),
        count: 1,
    })
}
fn set_component(stack: &mut ItemStack, name: &str, value: &Value) {
    let remove = name.starts_with('!');
    if let Some(id) = DataComponent::try_from_name(name.trim_start_matches('!')) {
        let component = if remove {
            None
        } else {
            read_loot_component(id, value)
        };
        if !remove && component.is_none() {
            return;
        }
        if let Some((_, existing)) = stack.patch.iter_mut().find(|(key, _)| *key == id) {
            *existing = component;
        } else {
            stack.patch.push((id, component));
        }
    }
}

fn apply_bonus_count(
    p: &Value,
    stack: &mut LootStack,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
) {
    let field = |name| p.get(name).unwrap_or(&Value::Null);

    // ApplyBonusCount.run uses TOOL, not the killer's looting enchantment.
    if params.tool.is_none() {
        return;
    }
    let level = enchantment_level(
        params.tool.as_ref(),
        p.get("enchantment")
            .and_then(Value::as_str)
            .unwrap_or("minecraft:fortune"),
    );
    let parameters = field("parameters");
    let formula = p
        .get("formula")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim_start_matches("minecraft:");
    stack.count = match formula {
        "ore_drops" if level > 0 => stack
            .get_count()
            .saturating_mul((rng.next_bounded_i32(level.saturating_add(2)) - 1).max(0) + 1),
        "uniform_bonus_count" => {
            let multiplier = parameters
                .get("bonusMultiplier")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .clamp(0, i64::from(MAX_LOOT_ROLLS)) as i32;
            stack.get_count().saturating_add(
                rng.next_bounded_i32(level.saturating_mul(multiplier).saturating_add(1)),
            )
        }
        "binomial_with_bonus_count" => {
            let extra = parameters.get("extra").and_then(Value::as_i64).unwrap_or(0) as i32;
            let probability = parameters
                .get("probability")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as f32;
            stack.get_count().saturating_add(
                (0..level.saturating_add(extra).clamp(0, MAX_LOOT_ROLLS))
                    .filter(|_| rng.next_f32() < probability)
                    .count() as i32,
            )
        }
        _ => stack.get_count(),
    };
}

fn copy_components(p: &Value, stack: &mut LootStack, params: &LootContextParameters) {
    // CopyComponentsFunction.run applies include and exclude to source components.
    let source = p.get("source").and_then(Value::as_str).unwrap_or_default();
    let include = p.get("include").and_then(Value::as_array);
    let exclude = p.get("exclude").and_then(Value::as_array);
    let selected = |name: &str| {
        include.is_none_or(|values| values.iter().any(|v| v.as_str() == Some(name)))
            && exclude.is_none_or(|values| !values.iter().any(|v| v.as_str() == Some(name)))
    };
    if source == "block_entity" {
        for (id, value) in &params.block_entity_components {
            if selected(id.to_name())
                && pumpkin_util::loot_table::component_copy_supported(id.to_name())
            {
                stack.stack.patch.retain(|(key, _)| key != id);
                stack.stack.patch.push((*id, Some(value.clone_dyn())));
            }
        }
    } else if source == "tool" {
        if let Some(tool) = &params.tool
            && !tool.is_empty()
        {
            for (id, _) in tool.item.components {
                if selected(id.to_name())
                    && pumpkin_util::loot_table::component_copy_supported(id.to_name())
                    && let Some(component) = item_component(tool, *id)
                {
                    stack.stack.patch.retain(|(key, _)| key != id);
                    stack.stack.patch.push((*id, Some(component.clone_dyn())));
                }
            }
            for (id, component) in &tool.patch {
                if selected(id.to_name())
                    && pumpkin_util::loot_table::component_copy_supported(id.to_name())
                    && let Some(component) = component
                {
                    stack.stack.patch.retain(|(key, _)| key != id);
                    stack.stack.patch.push((*id, Some(component.clone_dyn())));
                }
            }
        }
    } else if let Some(entity) = entity_target(source, params) {
        for (name, value) in &entity.components {
            if selected(name) {
                set_component(&mut stack.stack, name, value);
            }
        }
    }
}

pub(super) fn apply_functions(
    functions: &[LootFunction],
    stack: &mut LootStack,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
) {
    apply_functions_inner(functions, stack, params, rng, &mut BTreeSet::new(), 0);
}
fn apply_functions_inner(
    functions: &[LootFunction],
    stack: &mut LootStack,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
    visiting: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth >= MAX_LOOT_DEPTH {
        return;
    }
    for function in functions {
        if !rng.charge_work(1) {
            return;
        }
        // LootItemConditionalFunction.apply tests the modifier's own condition.
        if !check_condition(&function.condition, params, rng) {
            continue;
        }
        let p = &function.parameters;
        match function.kind.as_str() {
            "sequence" => {
                // SequenceFunction.run applies children after testing its own condition once.
                apply_functions_inner(&function.functions, stack, params, rng, visiting, depth + 1);
            }
            "reference" => {
                if let Some(name) = p.as_str() {
                    let name = registry_key(name);
                    if visiting.insert(name.clone()) {
                        if let Some(value) = super::registry::modifier(params, &name) {
                            apply_functions_inner(&value, stack, params, rng, visiting, depth + 1);
                        }
                        visiting.remove(&name);
                    }
                }
            }
            "copy_state" => run_copy_block_state(p, stack, params),
            "set_count" => run_set_item_count(p, stack, rng),
            "apply_bonus" => apply_bonus_count(p, stack, params, rng),
            "looting_enchant" | "enchanted_count_increase" => {
                run_enchanted_count_increase(p, stack, params, rng);
            }
            "explosion_decay" => {
                if let Some(radius) = params.explosion_radius {
                    // ApplyExplosionDecay.run rolls independently for each item.
                    stack.count = (0..stack.get_count().clamp(0, MAX_LOOT_ROLLS))
                        .filter(|_| rng.next_f32() <= 1.0 / radius)
                        .count() as i32;
                }
            }
            "limit_count" => run_limit_count(p, stack, rng),
            "set_components" => {
                if let Some(components) = p.get("components").and_then(Value::as_object) {
                    for (name, value) in components {
                        set_component(&mut stack.stack, name, value);
                    }
                }
            }
            "copy_components" => copy_components(p, stack, params),
            "furnace_smelt" if stack.get_count() > 0 => run_smelt_item(p, stack),
            _ => {}
        }
    }
}

fn run_copy_block_state(p: &Value, stack: &mut LootStack, params: &LootContextParameters) {
    // CopyBlockState.run uses only properties present on the configured block.
    let Some(configured_block) = p.get("block").and_then(Value::as_str).and_then(|name| {
        pumpkin_data::Block::from_registry_key(name.trim_start_matches("minecraft:"))
    }) else {
        return;
    };
    if let Some(state) = params.block_state {
        let block = state.id.to_block();
        if let Some(properties) = block.properties(state.id) {
            let selected = p.get("properties").and_then(Value::as_array);
            let mut values = stack
                .stack
                .get_data_component::<pumpkin_data::data_component_impl::BlockStateImpl>()
                .map(|v| v.properties.to_vec())
                .unwrap_or_default();
            for (name, value) in properties.to_props() {
                let property = pumpkin_data::loot_table::get_loot_property_type(block.name, name);
                let configured =
                    pumpkin_data::loot_table::get_loot_property_type(configured_block.name, name);
                if selected
                    .is_some_and(|selected| selected.iter().any(|v| v.as_str() == Some(name)))
                    && property
                        .zip(configured)
                        .is_some_and(|(actual, configured)| same_property_type(actual, configured))
                {
                    values.retain(|(key, _)| key.as_ref() != name);
                    values.push((
                        std::borrow::Cow::Borrowed(name),
                        std::borrow::Cow::Borrowed(value),
                    ));
                }
            }
            stack
                .stack
                .patch
                .retain(|(key, _)| *key != DataComponent::BlockState);
            stack.stack.patch.push((
                DataComponent::BlockState,
                Some(Box::new(
                    pumpkin_data::data_component_impl::BlockStateImpl {
                        properties: std::borrow::Cow::Owned(values),
                    },
                )),
            ));
        }
    }
}

fn same_property_type(
    actual: &pumpkin_data::loot_table::LootPropertyType,
    configured: &pumpkin_data::loot_table::LootPropertyType,
) -> bool {
    // Property.equals and its IntegerProperty/EnumProperty overrides include the value type and set.
    use pumpkin_data::loot_table::LootPropertyType;
    match (actual, configured) {
        (LootPropertyType::Boolean, LootPropertyType::Boolean) => true,
        (LootPropertyType::Integer(a, b), LootPropertyType::Integer(c, d)) => a == c && b == d,
        (LootPropertyType::Enum(a), LootPropertyType::Enum(b)) => a == b,
        _ => false,
    }
}

fn run_set_item_count(p: &Value, stack: &mut LootStack, rng: &mut LootRandom<'_>) {
    let field = |name| p.get(name).unwrap_or(&Value::Null);
    let Some(count) = number_int(field("count"), rng, 0) else {
        return;
    };
    stack.count = if p.get("add").and_then(Value::as_bool).unwrap_or(false) {
        stack.get_count().saturating_add(count)
    } else {
        count
    };
}

fn run_enchanted_count_increase(
    p: &Value,
    stack: &mut LootStack,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
) {
    let field = |name| p.get(name).unwrap_or(&Value::Null);
    // EnchantedCountIncreaseFunction.run rounds a FLOAT provider times level.
    let level = attacker_enchantment_level(
        params,
        p.get("enchantment")
            .and_then(Value::as_str)
            .unwrap_or("minecraft:looting"),
    );
    if level == 0 {
        return;
    }
    let Some(count) = number_float(field("count"), rng, 0) else {
        return;
    };
    stack.count = stack
        .get_count()
        .saturating_add((level as f32 * count + 0.5).floor() as i32);
    if let Some(limit) = p.get("limit").and_then(Value::as_i64).filter(|v| *v > 0) {
        stack.count = stack.get_count().min(limit as i32);
    }
}

fn run_limit_count(p: &Value, stack: &mut LootStack, rng: &mut LootRandom<'_>) {
    let field = |name| p.get(name).unwrap_or(&Value::Null);
    // LimitCount.run uses an IntRange with independently optional bounds.
    let limit = field("limit");
    if let Some(exact) = limit.as_i64() {
        stack.count = exact as i32;
    } else {
        let bounds_supported = ["min", "max"].iter().all(|name| {
            limit
                .get(name)
                .is_none_or(|v| pumpkin_util::loot_table::number_provider_supported(v, 0))
        });
        if !limit.is_object() || !bounds_supported {
            return;
        }
        let mut count = stack.get_count();
        if let Some(min) = limit.get("min").and_then(|v| number_int(v, rng, 0)) {
            count = count.max(min);
        }
        if let Some(max) = limit.get("max").and_then(|v| number_int(v, rng, 0)) {
            count = count.min(max);
        }
        stack.count = count;
    }
}

fn run_smelt_item(p: &Value, stack: &mut LootStack) {
    // SmeltItemFunction.run looks up a smelting recipe and preserves input count.
    if let Some(recipe) = pumpkin_data::recipes::get_cooking_recipe_with_ingredient(
        stack.stack.item,
        pumpkin_data::recipes::CookingRecipeKind::Smelting,
    ) && let Some(mut result) = new_stack(recipe.result.id)
    {
        let input_count = if p
            .get("use_input_count")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            stack.count
        } else {
            1
        };
        result.count = input_count
            .saturating_mul(i32::from(recipe.result.count))
            .min(i32::from(result.stack.get_max_stack_size()));
        *stack = result;
    }
}
