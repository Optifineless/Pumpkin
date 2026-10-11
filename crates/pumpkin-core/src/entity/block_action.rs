use super::player::Player;
use crate::{
    command::{snbt::SnbtParser, string_reader::StringReader},
    world::{
        World,
        loot::{LootContextParameters, match_block},
    },
};
use pumpkin_codecs::{DynamicOps, json_ops::JsonOps};
use pumpkin_data::data_component_impl::CanBreakImpl;
use pumpkin_nbt::{NbtCompound, nbt_ops::NbtOps, tag::NbtTag};
use pumpkin_util::{GameMode, math::position::BlockPos};

impl Player {
    /// Authorizes a dig before block attack callbacks, including cancellable damage events.
    pub(crate) fn may_attack_block(
        self: &std::sync::Arc<Self>,
        world: &World,
        position: &BlockPos,
    ) -> bool {
        // ServerPlayerGameMode.handleBlockBreakAction checks protection before BlockState.attack.
        if !world.is_in_build_limit(*position)
            || world.is_in_spawn_protection(self, position)
            || !world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(f64::from(position.0.x), f64::from(position.0.z))
            || self.block_action_restricted(world, position)
        {
            return false;
        }
        if let Some(server) = world.server.upgrade() {
            let mut event = crate::plugin::api::events::block::block_damage::BlockDamageEvent::new(
                self.clone(),
                world.get_block(position),
                *position,
                false,
            );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
        }
        true
    }

    /// Checks spectator and adventure restrictions before a block attack.
    pub(crate) fn block_action_restricted(&self, world: &World, position: &BlockPos) -> bool {
        // Player.blockActionRestricted -> AdventureModePredicate.test -> BlockPredicate.matches(BlockInWorld).
        match self.gamemode.load() {
            GameMode::Spectator => true,
            GameMode::Adventure if !self.may_build() => {
                let held = self.inventory().held_item();
                held.is_empty()
                    || held
                        .get_data_component::<CanBreakImpl>()
                        .is_none_or(|predicate| match &predicate.predicate {
                            NbtTag::List(predicates) => {
                                !predicates.iter().any(|p| matches(p, world, position))
                            }
                            predicate => !matches(predicate, world, position),
                        })
            }
            _ => false,
        }
    }
}

fn matches(predicate: &NbtTag, world: &World, position: &BlockPos) -> bool {
    let NbtTag::Compound(fields) = predicate else {
        return false;
    };
    let json = NbtOps.convert_to(&JsonOps, predicate.clone());
    if !match_block(
        &json["blocks"],
        &json["state"],
        &LootContextParameters {
            block_state: Some(world.get_block_state(position)),
            ..Default::default()
        },
    ) {
        return false;
    }
    let Some(expected) = fields.get("nbt") else {
        return true;
    };
    let expected = match expected {
        NbtTag::String(text) => {
            let Ok(tag) = SnbtParser::parse_for_commands(&mut StringReader::new(text.to_string()))
            else {
                return false;
            };
            tag
        }
        tag => tag.clone(),
    };
    let Some(entity) = world.get_block_entity(position) else {
        return false;
    };
    let mut actual = NbtCompound::new();
    entity.write_internal(&mut actual);
    crate::command::argument_types::entity_selector::matches_nbt(
        &expected,
        &NbtTag::Compound(actual),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        entity::death_test_world::DeathTestWorld,
        net::java::combat_test_support::TestPlayer,
        world::spawn_test_support::{proto, publish},
    };
    use pumpkin_data::{Block, item::Item, item_stack::ItemStack};

    #[tokio::test]
    async fn adventure_block_action_requires_matching_predicate() {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(
            &world,
            proto(&pumpkin_data::biome::Biome::PLAINS, &Block::STONE),
        );
        let player = TestPlayer::new(&world).player;
        player.gamemode.store(GameMode::Adventure);
        player.abilities.lock().unwrap().allow_modify_world = false;
        let pos = BlockPos::new(8, 63, 8);
        assert!(player.block_action_restricted(&world, &pos));
        let mut fields = NbtCompound::new();
        fields.put_string("blocks", "minecraft:stone".to_owned());
        let mut tool = ItemStack::new(1, &Item::STICK);
        tool.set_data_component(CanBreakImpl {
            predicate: NbtTag::Compound(fields),
        });
        player
            .inventory()
            .set_stack_in_hand(pumpkin_util::Hand::Right, tool);
        assert!(!player.block_action_restricted(&world, &pos));
        assert!(player.block_action_restricted(&world, &pos.up()));
        assert!(
            !crate::command::argument_types::entity_selector::matches_nbt(
                &NbtTag::List(Vec::new()),
                &NbtTag::List(vec![NbtTag::Int(1)])
            )
        );
        fixture.server.shutdown().await;
        crate::server::fixture_lifecycle::finish().await;
    }
}
