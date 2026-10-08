use super::FishingBobberEntity;
use crate::{
    entity::{
        Entity, EntityBase,
        experience_orb::ExperienceOrbEntity,
        item::ItemEntity,
        player::{Player, advancement::trigger::AdvancementTrigger},
    },
    plugin::api::events::{
        entity::item_spawn::ItemSpawnEvent,
        player::fish::{PlayerFishEvent, PlayerFishState},
    },
    world::loot::{LootContextParameters, build_fishing_loot_context},
};
use pumpkin_data::{
    entity::{EntityStatus, EntityType},
    item_stack::ItemStack,
    statistic::{CustomStatistic, StatisticCategory},
    tag::{self, Taggable},
};
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

impl FishingBobberEntity {
    /// Builds `FishingHook.retrieve`'s FISHING context with the actual hook, tool and owner luck.
    #[must_use]
    pub fn fishing_loot_context(&self, owner: &Player, rod: &ItemStack) -> LootContextParameters {
        build_fishing_loot_context(self, owner, rod, self.luck)
    }

    // FishingHook.retrieve: grounded overrides the catch's cost; an empty reel costs nothing.
    pub fn reel_in(&self, _player: &Player, rod: &ItemStack, hand: Hand) -> i32 {
        if self.entity.is_removed() {
            return 0;
        }
        // FishingHook.retrieve resolves getPlayerOwner before shouldStopFishing.
        let owner = self.get_player_owner();
        let Some(player) = owner.as_deref() else {
            return 0;
        };
        if self.should_stop_fishing(player) {
            self.discard();
            return 0;
        }
        let world = self.entity.world.load_full();
        let hooked = world.get_entity_by_id(self.hooked_entity_id.load(Relaxed));
        let damage = if let Some(hooked) = hooked {
            if self
                .fire_fish_event(
                    PlayerFishState::CaughtEntity,
                    Some(hooked.as_ref()),
                    hand,
                    0,
                )
                .is_none()
            {
                return 0;
            }
            // FishingHook.pullEntity: the client simulates player motion via event 31.
            if hooked.get_player().is_none() {
                hooked
                    .get_entity()
                    .add_velocity((player.position() - self.entity.pos.load()) * 0.1);
            }
            world.send_entity_status(&self.entity, EntityStatus::FishingRodReelIn, None);
            if let Some(item) = hooked.cast_any().downcast_ref::<ItemEntity>() {
                let item_id = item
                    .get_item_stack()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .item
                    .registry_key;
                player.trigger_advancement(AdvancementTrigger::FishedItem {
                    item_id: format!("minecraft:{item_id}"),
                });
            }
            if hooked.get_entity().entity_type == &EntityType::ITEM {
                3
            } else {
                5
            }
        } else if self.bite_countdown.load(Relaxed) > 0 {
            let context = self.fishing_loot_context(player, rod);
            let items = world
                .get_loot_table("minecraft:gameplay/fishing")
                .map_or_else(Vec::new, |table| {
                    table.generate_loot_with_context(0, &context)
                });
            for stack in items {
                if !self.spawn_catch(player, stack, hand) {
                    return 0;
                }
            }
            1
        } else {
            if self
                .fire_fish_event(PlayerFishState::ReelIn, None, hand, 0)
                .is_none()
            {
                return 0;
            }
            0
        };
        let damage = if self.entity.on_ground.load(Relaxed) {
            if self
                .fire_fish_event(PlayerFishState::InGround, None, hand, 0)
                .is_none()
            {
                return 0;
            }
            2
        } else {
            damage
        };
        self.discard();
        damage
    }

    fn spawn_catch(&self, player: &Player, stack: ItemStack, hand: Hand) -> bool {
        let world = self.entity.world.load_full();
        let pos = self.entity.pos.load();
        let entity = Entity::new(world.clone(), pos, &EntityType::ITEM);
        let item_type = stack.item;
        let item = ItemEntity::new_with_velocity(
            entity,
            stack,
            catch_velocity(player.position() - pos),
            0,
        );
        let Some(xp) = self.fire_fish_event(
            PlayerFishState::CaughtFish,
            Some(&item),
            hand,
            rand::random_range(1..=6),
        ) else {
            return false;
        };
        player.trigger_advancement(AdvancementTrigger::FishedItem {
            item_id: format!("minecraft:{}", item_type.registry_key),
        });
        let mut spawn_event = ItemSpawnEvent::new(
            item.get_entity().entity_id,
            pos,
            item_type.registry_key.to_string(),
        );
        if let Some(server) = world.server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut spawn_event);
        }
        if !spawn_event.cancelled {
            world.spawn_entity(Arc::new(item));
        }
        if xp > 0 {
            // FishingHook.retrieve creates one orb carrying 1..=6 points per loot stack.
            let orb = Entity::new(
                world.clone(),
                player.position().add_raw(0.0, 0.5, 0.5),
                &EntityType::EXPERIENCE_ORB,
            );
            // ExperienceOrb constructor launch motion awaits the XP-orb branch.
            world.spawn_entity(Arc::new(ExperienceOrbEntity::new(orb, xp as u32)));
        }
        if item_type.has_tag(&tag::Item::MINECRAFT_FISHES) {
            player.increment_stat(
                StatisticCategory::Custom,
                CustomStatistic::FishCaught as i32,
                1,
            );
        }
        true
    }

    pub(super) fn fire_fish_event(
        &self,
        state: PlayerFishState,
        caught: Option<&dyn EntityBase>,
        hand: Hand,
        xp: i32,
    ) -> Option<i32> {
        let world = self.entity.world.load();
        let Some(owner) = self.get_player_owner() else {
            return Some(xp);
        };
        let Some(server) = world.server.upgrade() else {
            return Some(xp);
        };
        let mut event = PlayerFishEvent::new(
            owner,
            caught.map(|entity| entity.get_entity().entity_uuid),
            self.entity.entity_uuid,
            caught.map_or_else(String::new, |entity| {
                entity.get_entity().entity_type.resource_name.to_string()
            }),
            state,
            hand,
            xp,
        );
        server.plugin_manager.fire_blocking(&server, &mut event);
        (!event.cancelled).then_some(event.exp_to_drop)
    }
}

// FishingHook.retrieve gives caught items the fourth root of squared distance as vertical lift.
pub(super) fn catch_velocity(delta: Vector3<f64>) -> Vector3<f64> {
    (delta * 0.1).add_raw(0.0, delta.length_squared().sqrt().sqrt() * 0.08, 0.0)
}
