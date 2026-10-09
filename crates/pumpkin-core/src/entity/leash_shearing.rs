use std::sync::Arc;

use pumpkin_data::{
    game_event::GameEvent,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};

use super::{Entity, player::Player};

// Leashable.leashableInArea uses a 32-block-wide box centered on the holder's bounds.
const LEASH_SEARCH_SIZE: f64 = 32.0;

impl Entity {
    // Leashable.dropLeash preserves the fork's events and atomically claims the link.
    pub(super) fn unleash_if_leashed(&self) -> bool {
        let world = self.world.load();
        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::entity::entity_unleash::EntityUnleashEvent::new(
                    self.entity_id,
                    "unleashed".to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
        }

        let old_holder = self
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if old_holder.is_none() {
            return false;
        }

        if let Some(holder) = &old_holder
            && let Some(server) = world.server.upgrade()
            && let Some(player) = holder.get_player()
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
        {
            let mut event = crate::plugin::api::events::player::player_unleash_entity::PlayerUnleashEntityEvent {
                player: player_arc,
                entity_id: self.entity_id,
                cancelled: false,
            };
            server.plugin_manager.fire_blocking(&server, &mut event);
        }

        let je_packet =
            pumpkin_protocol::java::client::play::CSetEntityLink::new(self.entity_id, -1, true);
        let be_packet = pumpkin_protocol::bedrock::client::CSetActorLink {
            link: pumpkin_protocol::bedrock::client::common::ActorLink {
                ridden_unique_id: pumpkin_protocol::codec::var_long::VarLong(self.entity_id as i64),
                rider_unique_id: pumpkin_protocol::codec::var_long::VarLong(-1),
                link_type: 0, // Unlink
                immediate: true,
                rider_initiated: false,
                vehicle_angular_velocity: 0.0,
            },
        };

        self.world.load().broadcast_to_chunk_editioned(
            self.chunk_pos.load(),
            &je_packet,
            &be_packet,
        );
        true
    }

    /// Cuts this entity's incoming and nearby outgoing leash connections.
    pub(crate) fn shear_off_all_leash_connections(&self, player: Option<&Player>) -> bool {
        // Entity.dropAllLeashConnections / shearOffAllLeashConnections.
        let world = self.world.load_full();
        let bounds = self.bounding_box.load();
        let center = (bounds.min + bounds.max) * 0.5;
        let radius = Vector3::new(
            LEASH_SEARCH_SIZE / 2.0,
            LEASH_SEARCH_SIZE / 2.0,
            LEASH_SEARCH_SIZE / 2.0,
        );
        let others = world.get_entities_at_box(&BoundingBox::new(center - radius, center + radius));
        let mut dropped = drop_lead(self);
        for other in others {
            let raw = other.get_entity();
            let attached = raw
                .leashed_to
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(|holder| holder.get_entity().entity_id == self.entity_id);
            if attached {
                dropped |= drop_lead(raw);
            }
        }
        if dropped {
            world.emit_game_event(GameEvent::Shear.name(), self.pos.load());
            let source = if player.is_some() {
                SoundCategory::Players
            } else if self.entity_type.category == &pumpkin_data::entity::MobCategory::MONSTER {
                SoundCategory::Hostile
            } else {
                SoundCategory::Neutral
            };
            world.play_sound(Sound::ItemShearsSnip, source, &self.pos.load());
        }
        dropped
    }
}

fn drop_lead(entity: &Entity) -> bool {
    if !entity.is_leashed() || !entity.unleash_if_leashed() {
        return false;
    }
    // Leashable.dropLeash -> Entity.spawnAtLocation drops at the entity's exact position.
    let world = entity.world.load_full();
    let position = entity.pos.load();
    let dropped = Entity::new(
        world.clone(),
        position,
        &pumpkin_data::entity::EntityType::ITEM,
    );
    let mut event = crate::plugin::api::events::entity::item_spawn::ItemSpawnEvent::new(
        dropped.entity_id,
        position,
        Item::LEAD.registry_key.to_string(),
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    if !event.cancelled {
        world.spawn_entity(Arc::new(super::item::ItemEntity::new(
            dropped,
            ItemStack::new(1, &Item::LEAD),
        )));
    }
    true
}

pub(super) fn shear_leashes_by_player(
    entity: &Entity,
    player: &Arc<Player>,
    tool: &mut ItemStack,
) -> bool {
    if tool.item != &Item::SHEARS || !entity.shear_off_all_leash_connections(Some(player)) {
        return false;
    }
    player.damage_detached_item(tool, 1);
    true
}

#[cfg(test)]
#[path = "leash_shearing_tests.rs"]
mod tests;
