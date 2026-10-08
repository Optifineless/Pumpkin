use super::super::Player;
use pumpkin_data::{
    Advancement, Block, BlockStateId, data_component_impl::JukeboxPlayableImpl,
    item_stack::ItemStack, tag::Taggable,
};
use pumpkin_util::math::position::BlockPos;
use serde_json::Value;
use std::sync::LazyLock;

struct BlockUseCriterion {
    advancement: &'static Advancement,
    name: &'static str,
    event: &'static str,
    conditions: Value,
}

static BLOCK_USE_CRITERIA: LazyLock<Vec<BlockUseCriterion>> = LazyLock::new(|| {
    Advancement::get_identifier_list()
        .iter()
        .filter_map(|id| Advancement::from_minecraft_name(&id.to_string()))
        .flat_map(|advancement| {
            advancement
                .action_criteria
                .iter()
                .filter_map(move |(name, event, conditions)| {
                    parse_conditions(conditions)
                        .ok()
                        .map(|conditions| BlockUseCriterion {
                            advancement,
                            name,
                            event,
                            conditions,
                        })
                })
        })
        .collect()
});

static PLANTING_CRITERIA: LazyLock<Vec<&'static BlockUseCriterion>> = LazyLock::new(|| {
    BLOCK_USE_CRITERIA
        .iter()
        .filter(|criterion| {
            criterion.event == "minecraft:placed_block"
                && (criterion.advancement == Advancement::HUSBANDRY_PLANT_SEED
                    || criterion.advancement == Advancement::HUSBANDRY_PLANT_ANY_SNIFFER_SEED)
        })
        .collect()
});

fn parse_conditions(conditions: &str) -> Result<Value, serde_json::Error> {
    #[cfg(test)]
    tests::JSON_PARSES.with(|count| count.set(count.get() + 1));
    serde_json::from_str(conditions)
}

impl Player {
    // ItemUsedOnLocationTrigger.trigger evaluates the pre-consumption tool and location context.
    pub(super) fn trigger_block_use(
        &self,
        trigger: &str,
        position: BlockPos,
        item: Option<&ItemStack>,
        state: BlockStateId,
    ) {
        for criterion in BLOCK_USE_CRITERIA.iter() {
            let conditions = &criterion.conditions;
            if criterion.event == trigger
                && conditions.get("player").is_none()
                && conditions.get("location").is_none_or(|condition| {
                    self.block_use_condition(condition, position, item, state)
                })
            {
                self.trigger_advancement_criterion(criterion.advancement, criterion.name);
            }
        }
    }

    // ItemUsedOnLocationTrigger (PLACED_BLOCK) reuses the loaded advancement predicates.
    pub(super) fn trigger_planting_criteria(&self, block_id: &str) {
        for criterion in PLANTING_CRITERIA.iter() {
            #[cfg(test)]
            tests::CRITERIA_VISITS.with(|count| count.set(count.get() + 1));
            if criterion.conditions["location"]["blocks"].as_str() == Some(block_id) {
                self.trigger_advancement_criterion(criterion.advancement, criterion.name);
            }
        }
    }

    fn block_use_condition(
        &self,
        condition: &Value,
        position: BlockPos,
        item: Option<&ItemStack>,
        state: BlockStateId,
    ) -> bool {
        match condition["type"].as_str() {
            Some("minecraft:all_of") => condition["terms"].as_array().is_some_and(|terms| {
                terms
                    .iter()
                    .all(|term| self.block_use_condition(term, position, item, state))
            }),
            Some("minecraft:any_of") => condition["terms"].as_array().is_some_and(|terms| {
                terms
                    .iter()
                    .any(|term| self.block_use_condition(term, position, item, state))
            }),
            Some("minecraft:match_block") => matches_block(condition, state),
            Some("minecraft:match_tool") => item.is_some_and(|item| {
                let predicate = &condition["predicate"];
                predicate.get("items").is_none_or(|items| {
                    registry_matches(items, item.item.registry_key, &|tag| {
                        item.item.is_tagged_with(tag).unwrap_or(false)
                    })
                }) && predicate.get("predicates").is_none_or(|components| {
                    components.as_object().is_some_and(|components| {
                        components.iter().all(|(name, value)| {
                            name == "minecraft:jukebox_playable"
                                && value.as_object().is_some_and(serde_json::Map::is_empty)
                                && item.get_data_component::<JukeboxPlayableImpl>().is_some()
                        })
                    })
                })
            }),
            Some("minecraft:location_check") => {
                let offset = position.offset(pumpkin_util::math::vector3::Vector3::new(
                    condition["offsetX"].as_i64().unwrap_or(0) as i32,
                    condition["offsetY"].as_i64().unwrap_or(0) as i32,
                    condition["offsetZ"].as_i64().unwrap_or(0) as i32,
                ));
                let state = if offset == position {
                    state
                } else {
                    self.world().get_block_state_id(&offset)
                };
                let predicate = &condition["predicate"];
                predicate
                    .get("block")
                    .is_none_or(|block| matches_block(block, state))
                    && predicate.get("smokey").is_none_or(|smokey| {
                        smokey.as_bool()
                            == Some(crate::block::blocks::beehive::is_smokey_pos(
                                &self.world(),
                                offset,
                            ))
                    })
                    && predicate.get("biomes").is_none_or(|biomes| {
                        registry_matches(
                            biomes,
                            self.world().get_biome(&offset).registry_id,
                            &|_| false,
                        )
                    })
            }
            _ => false,
        }
    }
}

fn registry_matches(value: &Value, name: &str, tagged: &dyn Fn(&str) -> bool) -> bool {
    if let Some(names) = value.as_array() {
        return names
            .iter()
            .any(|value| registry_matches(value, name, tagged));
    }
    value.as_str().is_some_and(|expected| {
        expected.strip_prefix('#').map_or_else(
            || {
                expected.strip_prefix("minecraft:").unwrap_or(expected)
                    == name.strip_prefix("minecraft:").unwrap_or(name)
            },
            tagged,
        )
    })
}

fn matches_block(predicate: &Value, state: BlockStateId) -> bool {
    let block = Block::from_state_id(state);
    predicate.get("blocks").is_none_or(|blocks| {
        registry_matches(blocks, block.name, &|tag| {
            block.is_tagged_with(tag).unwrap_or(false)
        })
    }) && predicate.get("state").is_none_or(|expected| {
        expected.as_object().is_some_and(|expected| {
            let properties = block
                .properties(state)
                .map(|properties| properties.to_props())
                .unwrap_or_default();
            expected.iter().all(|(name, value)| {
                properties
                    .iter()
                    .any(|(key, actual)| *key == name && value.as_str() == Some(actual))
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
    use std::cell::Cell;
    thread_local! {
        pub(super) static JSON_PARSES: Cell<usize> = const { Cell::new(0) };
        pub(super) static CRITERIA_VISITS: Cell<usize> = const { Cell::new(0) };
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn review_planting_reuses_parsed_criteria() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        fixture.player.advancements.try_lock().unwrap().player =
            std::sync::Arc::downgrade(&fixture.player);
        // Include initial loading outside the per-placement measurement.
        fixture.player.trigger_planting_criteria("minecraft:air");
        JSON_PARSES.set(0);
        CRITERIA_VISITS.set(0);
        fixture
            .player
            .trigger_planting_criteria("minecraft:torchflower_crop");
        assert_eq!(
            CRITERIA_VISITS.get(),
            9,
            "criterion iterations per placement"
        );
        assert_eq!(JSON_PARSES.get(), 0, "JSON parses per placement");
        assert!(
            fixture
                .player
                .has_advancement(Advancement::HUSBANDRY_PLANT_SEED)
        );
        assert!(
            fixture
                .player
                .has_advancement(Advancement::HUSBANDRY_PLANT_ANY_SNIFFER_SEED)
        );
        assert!(world.level.shutdown().await.is_ok());
    }
}
