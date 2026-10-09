use pumpkin_data::{item::Item, item_stack::ItemStack};
pub use pumpkin_util::loot_table::{
    DynamicLootCondition, DynamicLootEntry, DynamicLootPool, DynamicLootTable, LootBonusFormula,
    LootCondition, LootEntry, LootEntryKind, LootFunction, LootPool, LootTable, LootTableReference,
};
mod random;
mod registry;
use random::{LootRandom, with_random};
use registry::{item_holders, registry_key, resolve_table, table_holders};
use std::collections::BTreeSet;
mod block_context;
mod context;
mod fishing_context;
pub use block_context::{build_block_loot_context, collect_block_entity_components};
pub use fishing_context::build_fishing_loot_context;
mod conditions;
mod container;
#[cfg(test)]
mod context_tests;
mod functions;
mod number_provider;
mod predicates;
#[cfg(test)]
mod production_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod second_review_tests;
#[cfg(test)]
mod tests;
use conditions::check_condition;
pub use container::{
    fill_chest_inventory, fill_chest_inventory_handle, fill_chest_inventory_with_context,
};
pub use context::{
    EntityLootState, LootContextParameters, build_command_kill_loot_context,
    build_container_loot_context, build_entity_death_loot_context, build_shearing_loot_context,
};
use functions::{LootStack, apply_functions, new_stack};
use number_provider::{number_float, number_int};
use predicates::{
    attacker_enchantment_level, enchantment_level, entity_predicate, entity_target, item_component,
    item_predicate, read_loot_component,
};
const MAX_LOOT_DEPTH: usize = 64;
const MAX_LOOT_ROLLS: i32 = 4096;
const MAX_LOOT_STACKS: usize = 4096;

struct ExpandedEntry<'a> {
    entry: &'a LootEntry,
    choice: EntryChoice,
    modifiers: Vec<&'a [LootFunction]>,
    weight: i32,
}
enum EntryChoice {
    All,
    Item(String),
    Table(usize, Option<String>),
}
struct LootTraversal {
    visiting: BTreeSet<usize>,
    remaining_rolls: usize,
}
fn expand<'a>(
    entry: &'a LootEntry,
    inherited: &[&'a [LootFunction]],
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
    output: &mut Vec<ExpandedEntry<'a>>,
) -> bool {
    if !rng.charge_work(1) || !check_condition(&entry.condition, params, rng) {
        return false;
    }
    let mut modifiers = vec![entry.functions.as_slice()];
    modifiers.extend_from_slice(inherited);
    // AlternativesEntry.compose, EntryGroup.compose and SequentialEntry.compose.
    match &entry.kind {
        LootEntryKind::Alternatives(children) => children
            .iter()
            .any(|child| expand(child, &modifiers, params, rng, output)),
        LootEntryKind::Sequence(children) => children
            .iter()
            .all(|child| expand(child, &modifiers, params, rng, output)),
        LootEntryKind::Group(children) => {
            if let [child] = children.as_slice() {
                return expand(child, &modifiers, params, rng, output);
            }
            for child in children {
                expand(child, &modifiers, params, rng, output);
            }
            true
        }
        LootEntryKind::Unsupported => false,
        LootEntryKind::Tag {
            items,
            expand: true,
        } => {
            let Some(items) = item_holders(items, rng) else {
                return false;
            };
            for item in items {
                push_expanded(entry, EntryChoice::Item(item), &modifiers, params, output);
            }
            true
        }
        LootEntryKind::Tables {
            tables,
            expand: true,
        } => {
            let mut choices = Vec::new();
            for (index, table) in tables.iter().enumerate() {
                match table {
                    LootTableReference::Named(name) => {
                        let Some(names) = table_holders(name, params, rng) else {
                            return false;
                        };
                        choices.extend(
                            names
                                .into_iter()
                                .map(|name| EntryChoice::Table(index, Some(name))),
                        );
                    }
                    LootTableReference::Inline(_) => choices.push(EntryChoice::Table(index, None)),
                }
                if choices.len() > MAX_LOOT_STACKS {
                    return false;
                }
            }
            for choice in choices {
                push_expanded(entry, choice, &modifiers, params, output);
            }
            true
        }
        LootEntryKind::Tag {
            items,
            expand: false,
        } if item_holders(items, rng).is_none() => false,
        LootEntryKind::Tables {
            tables,
            expand: false,
        } if tables.iter().any(|table| match table {
            LootTableReference::Named(name) => table_holders(name, params, rng).is_none(),
            LootTableReference::Inline(_) => false,
        }) =>
        {
            false
        }
        _ => {
            push_expanded(entry, EntryChoice::All, &modifiers, params, output);
            true
        }
    }
}
fn push_expanded<'a>(
    entry: &'a LootEntry,
    choice: EntryChoice,
    modifiers: &[&'a [LootFunction]],
    params: &LootContextParameters,
    output: &mut Vec<ExpandedEntry<'a>>,
) {
    // UniformContainerBase.EntryBase.getWeight includes quality and luck.
    let weight = (entry.weight as f32 + entry.quality as f32 * params.luck).floor() as i32;
    if weight > 0 && output.len() < MAX_LOOT_STACKS {
        output.push(ExpandedEntry {
            entry,
            choice,
            modifiers: modifiers.to_vec(),
            weight,
        });
    }
}
fn create_stacks(
    candidate: &ExpandedEntry<'_>,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
    traversal: &mut LootTraversal,
    depth: usize,
    output: &mut dyn FnMut(LootStack, &mut LootRandom<'_>),
) {
    // NestedLootTable.createItemStack -> adjustOutput streams each item through enclosing modifiers.
    let mut adjusted = |mut stack: LootStack, rng: &mut LootRandom<'_>| {
        for functions in &candidate.modifiers {
            apply_functions(functions, &mut stack, params, rng);
        }
        output(stack, rng);
    };
    match &candidate.entry.kind {
        LootEntryKind::Item(name) => {
            if let Some(stack) = new_stack(name) {
                adjusted(stack, rng);
            }
        }
        LootEntryKind::Tag { items, .. } => {
            let items = match &candidate.choice {
                EntryChoice::Item(item) => vec![item.clone()],
                _ => item_holders(items, rng).unwrap_or_default(),
            };
            for item in items {
                if let Some(stack) = new_stack(&item) {
                    adjusted(stack, rng);
                }
            }
        }
        LootEntryKind::Tables { tables, .. } => {
            for (index, reference) in tables.iter().enumerate() {
                if matches!(&candidate.choice, EntryChoice::Table(selected, _) if *selected != index)
                {
                    continue;
                }
                match reference {
                    LootTableReference::Inline(table) => {
                        roll_table(table, params, rng, traversal, depth + 1, &mut adjusted);
                    }
                    LootTableReference::Named(name) => {
                        let names = match &candidate.choice {
                            EntryChoice::Table(_, Some(name)) => vec![name.clone()],
                            _ => table_holders(name, params, rng).unwrap_or_default(),
                        };
                        for name in names {
                            if let Some(table) = resolve_table(params, &name) {
                                roll_table(
                                    table.parsed(),
                                    params,
                                    rng,
                                    traversal,
                                    depth + 1,
                                    &mut adjusted,
                                );
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}
fn roll_table(
    table: &DynamicLootTable,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
    traversal: &mut LootTraversal,
    depth: usize,
    output: &mut dyn FnMut(LootStack, &mut LootRandom<'_>),
) {
    let identity = std::ptr::from_ref(table) as usize;
    if depth >= MAX_LOOT_DEPTH || !traversal.visiting.insert(identity) {
        return;
    }
    for pool in &table.pools {
        if !pumpkin_util::loot_table::number_provider_supported(&pool.rolls, 0)
            || !pumpkin_util::loot_table::number_provider_supported(&pool.bonus_rolls, 0)
            || !check_condition(&pool.condition, params, rng)
        {
            continue;
        }
        // LootPool.addRandomItems expands conditions EACH roll, before weighted selection.
        let Some(rolls) = number_int(&pool.rolls, rng, 0) else {
            continue;
        };
        let Some(bonus) = number_float(&pool.bonus_rolls, rng, 0) else {
            continue;
        };
        let rolls = rolls
            .saturating_add((bonus * params.luck).floor() as i32)
            .clamp(0, MAX_LOOT_ROLLS);
        for _ in 0..rolls {
            if traversal.remaining_rolls == 0 {
                break;
            }
            traversal.remaining_rolls -= 1;
            let mut candidates = Vec::new();
            for entry in &pool.entries {
                expand(entry, &[], params, rng, &mut candidates);
            }
            let total = candidates.iter().fold(0i32, |weight, candidate| {
                weight.saturating_add(candidate.weight)
            });
            if total == 0 {
                continue;
            }
            let mut pick = if candidates.len() == 1 {
                0
            } else {
                rng.next_bounded_i32(total)
            };
            for candidate in candidates {
                pick -= candidate.weight;
                if pick < 0 {
                    let mut adjusted = |mut stack: LootStack, rng: &mut LootRandom<'_>| {
                        apply_functions(&pool.functions, &mut stack, params, rng);
                        apply_functions(&table.functions, &mut stack, params, rng);
                        output(stack, rng);
                    };
                    create_stacks(&candidate, params, rng, traversal, depth, &mut adjusted);
                    break;
                }
            }
        }
    }
    traversal.visiting.remove(&identity);
}
fn generate_with_random(
    table: &DynamicLootTable,
    params: &LootContextParameters,
    rng: &mut LootRandom<'_>,
) -> Vec<ItemStack> {
    // Block.getDrops supplies TOOL=ItemStack.EMPTY for bare-handed and explosion drops.
    let mut params = std::borrow::Cow::Borrowed(params);
    if params.block_state.is_some() && params.tool.is_none() {
        params.to_mut().tool = Some(ItemStack::EMPTY.clone());
    }
    let mut result = Vec::new();
    let mut output = |mut stack: LootStack, _: &mut LootRandom<'_>| {
        // ItemStack.isEmpty/getCount: downstream consumers discard empty/AIR stacks.
        if stack.stack.item == &Item::AIR {
            return;
        }
        let max = i32::from(stack.stack.get_max_stack_size()).max(1);
        while stack.count > 0 && result.len() < MAX_LOOT_STACKS {
            let count = stack.count.min(max);
            let mut item = stack.stack.clone();
            item.item_count = count as u8;
            result.push(item);
            stack.count -= count;
        }
    };
    roll_table(
        table,
        &params,
        rng,
        &mut LootTraversal {
            visiting: BTreeSet::new(),
            remaining_rolls: MAX_LOOT_ROLLS as usize,
        },
        0,
        &mut output,
    );
    result
}
#[must_use]
pub fn generate_loot(table: &LootTable, seed: i64) -> Vec<ItemStack> {
    generate_loot_with_context(table, seed, &LootContextParameters::default())
}
#[must_use]
pub fn generate_loot_with_context(
    table: &LootTable,
    seed: i64,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    generate_dynamic_loot_with_context(table.parsed(), seed, params)
}
pub use generate_loot as generate_chest_loot;
#[must_use]
pub fn generate_dynamic_loot(table: &DynamicLootTable, seed: i64) -> Vec<ItemStack> {
    generate_dynamic_loot_with_context(table, seed, &LootContextParameters::default())
}
#[must_use]
pub fn generate_dynamic_loot_with_context(
    table: &DynamicLootTable,
    seed: i64,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    let mut result = Vec::new();
    with_random(table, seed, params, &mut |rng| {
        result = generate_with_random(table, params, rng);
    });
    result
}

/// A handle to either a compile-time static loot table or a datapack dynamic loot table.
#[derive(Clone, Debug)]
pub enum LootTableHandle {
    Static(&'static LootTable),
    Dynamic(std::sync::Arc<DynamicLootTable>),
}

impl From<&'static LootTable> for LootTableHandle {
    fn from(t: &'static LootTable) -> Self {
        Self::Static(t)
    }
}

impl From<std::sync::Arc<DynamicLootTable>> for LootTableHandle {
    fn from(t: std::sync::Arc<DynamicLootTable>) -> Self {
        Self::Dynamic(t)
    }
}

impl LootTableHandle {
    fn parsed(&self) -> &DynamicLootTable {
        match self {
            Self::Static(table) => table.parsed(),
            Self::Dynamic(table) => table,
        }
    }
    #[must_use]
    pub fn generate_loot(&self, seed: i64) -> Vec<ItemStack> {
        self.generate_loot_with_context(seed, &LootContextParameters::default())
    }

    #[must_use]
    pub fn generate_loot_with_context(
        &self,
        seed: i64,
        params: &LootContextParameters,
    ) -> Vec<ItemStack> {
        generate_loot_from_handle(self, seed, params)
    }
}

#[must_use]
pub fn get_loot_table(key: &str) -> Option<LootTableHandle> {
    let full_key = if key.contains(':') {
        key.to_string()
    } else {
        format!("minecraft:{key}")
    };
    pumpkin_data::loot_table::get_loot_table(key)
        .or_else(|| pumpkin_data::loot_table::get_loot_table(&full_key))
        .map(LootTableHandle::Static)
}

#[must_use]
pub fn generate_loot_from_handle(
    handle: &LootTableHandle,
    seed: i64,
    params: &LootContextParameters,
) -> Vec<ItemStack> {
    match handle {
        LootTableHandle::Static(table) => generate_loot_with_context(table, seed, params),
        LootTableHandle::Dynamic(table) => generate_dynamic_loot_with_context(table, seed, params),
    }
}

// Shared StatePropertiesPredicate matcher for adventure-mode block actions.
pub(crate) use predicates::match_block;
