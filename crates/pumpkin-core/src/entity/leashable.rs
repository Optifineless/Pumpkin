use crate::entity::Entity;
use crate::entity::EntityBase;
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use std::sync::Arc;

const LEASH_SEARCH_SIZE: f64 = 32.0;
const EXTENDED_LEASH_SNAP_DISTANCE: f64 = 16.0;

/// The outcome distinguishes plugin cancellation from a knot closing before the leash link commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeashToResult {
    Attached,
    Cancelled,
    HolderUnavailable,
}

/// Builds vanilla `Leashable.leashableInArea`'s 32-block cube centered on the holder point.
#[must_use]
pub fn leash_search_box(center: Vector3<f64>) -> BoundingBox {
    let half_size = LEASH_SEARCH_SIZE / 2.0;
    let radius = Vector3::new(half_size, half_size, half_size);
    BoundingBox::new(center - radius, center + radius)
}

/// Checks vanilla `Leashable.canHaveALeashAttachedTo` using the mob's own snap distance.
#[must_use]
pub fn can_have_a_leash_attached_to(entity: &Entity, holder: &Entity) -> bool {
    entity.entity_id != holder.entity_id
        && leash_distance(entity, holder) <= entity.leash_snap_distance()
}

/// Returns vanilla `Leashable.leashDistanceTo` between entity bounding-box centers.
#[must_use]
pub fn leash_distance(entity: &Entity, holder: &Entity) -> f64 {
    let entity_bounds = entity.bounding_box.load();
    let holder_bounds = holder.bounding_box.load();
    let difference = (entity_bounds.min + entity_bounds.max) * 0.5
        - (holder_bounds.min + holder_bounds.max) * 0.5;
    difference.length()
}

impl Entity {
    /// Mirrors `Leashable.setLeashedTo`, committing the link only while a knot can accept it.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the knot reservation borrows the holder while the pointer is replaced"
    )]
    pub fn leash_to(&self, holder: Arc<dyn EntityBase>) -> LeashToResult {
        let holder_entity_id = holder.get_entity().entity_id;
        let world = self.world.load();
        if let Some(server) = world.server.upgrade()
            && let Some(player) = holder.get_player()
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_leash_entity::PlayerLeashEntityEvent {
                    player: player_arc,
                    entity_id: self.entity_id,
                    holder_id: holder_entity_id,
                    cancelled: false,
                };
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return LeashToResult::Cancelled;
            }
        }

        let (old_holder, attachment) = {
            let attachment = if holder.get_entity().entity_type == &EntityType::LEASH_KNOT {
                let Some(knot) = holder
                    .cast_any()
                    .downcast_ref::<super::decoration::leash_knot::LeashKnotEntity>()
                else {
                    return LeashToResult::HolderUnavailable;
                };
                let Some(attachment) = knot.try_begin_attachment() else {
                    return LeashToResult::HolderUnavailable;
                };
                Some(attachment)
            } else {
                None
            };

            let old_holder = self
                .leashed_to
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(holder.clone());
            (old_holder, attachment)
        };
        // The reservation covers only the pointer mutation, never the packet sends or callbacks.
        drop(attachment);

        let je_packet = pumpkin_protocol::java::client::play::CSetEntityLink::new(
            self.entity_id,
            holder_entity_id,
            true,
        );
        let be_packet = pumpkin_protocol::bedrock::client::CSetActorLink {
            link: pumpkin_protocol::bedrock::client::common::ActorLink {
                ridden_unique_id: pumpkin_protocol::codec::var_long::VarLong(self.entity_id as i64),
                rider_unique_id: pumpkin_protocol::codec::var_long::VarLong(
                    holder_entity_id as i64,
                ),
                link_type: 1, // Leash link
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

        if let Some(old_holder) = old_holder
            && old_holder.get_entity().entity_id != holder_entity_id
        {
            super::decoration::leash_knot::LeashKnotEntity::notify_leashee_removed_from(
                old_holder.as_ref(),
            );
        }

        LeashToResult::Attached
    }

    /// Returns the vanilla `Leashable.leashSnapDistance` value for this entity type.
    #[must_use]
    pub(crate) fn leash_snap_distance(&self) -> f64 {
        if self.entity_type == &EntityType::HAPPY_GHAST || self.entity_type == &EntityType::GHAST {
            // HappyGhast#leashSnapDistance and Ghast#leashSnapDistance override the 12-block default.
            EXTENDED_LEASH_SNAP_DISTANCE
        } else {
            Self::LEASH_SNAP_DISTANCE
        }
    }
}
