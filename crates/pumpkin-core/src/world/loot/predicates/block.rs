use super::registry_matches;
use crate::world::loot::LootContextParameters;
use serde_json::Value;
pub(crate) fn match_block(
    blocks: &Value,
    properties: &Value,
    params: &LootContextParameters,
) -> bool {
    // MatchBlock.test and StatePropertiesPredicate.matches.
    let Some(state) = params.block_state else {
        return false;
    };
    let block = state.id.to_block();
    if !blocks.is_null() && !registry_matches(blocks, block) {
        return false;
    }
    let props = block
        .properties(state.id)
        .map(|p| p.to_props())
        .unwrap_or_default();
    properties.as_object().map_or_else(
        || properties.is_null(),
        |properties| {
            properties.iter().all(|(key, expected)| {
                props
                    .iter()
                    .find(|(name, _)| *name == key)
                    .is_some_and(|(_, actual)| {
                        let Some(property) =
                            pumpkin_data::loot_table::get_loot_property_type(block.name, key)
                        else {
                            return false;
                        };
                        let Some(actual) = property.value(actual) else {
                            return false;
                        };
                        if let Some(exact) = expected.as_str() {
                            return property.value(exact) == Some(actual);
                        }
                        let Some(range) = expected.as_object() else {
                            return false;
                        };
                        for (bound, minimum) in [("min", true), ("max", false)] {
                            if let Some(value) = range.get(bound) {
                                let Some(value) = value.as_str().and_then(|v| property.value(v))
                                else {
                                    return false;
                                };
                                if (minimum && actual < value) || (!minimum && actual > value) {
                                    return false;
                                }
                            }
                        }
                        true
                    })
            })
        },
    )
}
