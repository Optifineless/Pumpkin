use crate::entity::{Entity, EntityBase, living::LivingEntity};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::{boundingbox::BoundingBox, vector3::Vector3};
use std::sync::{Arc, atomic::Ordering::Relaxed};

const LEASH_SEARCH_SIZE: f64 = 32.0;
const EXTENDED_LEASH_SNAP_DISTANCE: f64 = 16.0;
const EXTENDED_LEASH_ELASTIC_DISTANCE: f64 = 10.0;

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

/// Checks vanilla `Leashable.canHaveALeashAttachedTo`, including species and range rules.
#[must_use]
pub fn can_have_a_leash_attached_to<E: EntityBase + ?Sized, H: EntityBase + ?Sized>(
    entity_base: &E,
    holder_base: &H,
) -> bool {
    let entity = entity_base.get_entity();
    let holder = holder_base.get_entity();
    entity.entity_id != holder.entity_id
        && can_have_a_leash_attached_to_position(entity_base, entity_center(holder))
}

/// Checks eligibility against a holder center before a not-yet-created leash knot is published.
#[must_use]
pub fn can_have_a_leash_attached_to_position<E: EntityBase + ?Sized>(
    entity_base: &E,
    holder_center: Vector3<f64>,
) -> bool {
    let entity = entity_base.get_entity();
    leash_distance_to_center(entity, holder_center) <= entity.leash_snap_distance()
        && entity_base
            .get_mob()
            .is_none_or(crate::entity::mob::Mob::can_be_leashed)
}

/// Returns vanilla `Leashable.leashDistanceTo` between entity bounding-box centers.
#[must_use]
pub fn leash_distance(entity: &Entity, holder: &Entity) -> f64 {
    leash_distance_to_center(entity, entity_center(holder))
}

fn entity_center(entity: &Entity) -> Vector3<f64> {
    let bounds = entity.bounding_box.load();
    (bounds.min + bounds.max) * 0.5
}

fn leash_distance_to_center(entity: &Entity, holder_center: Vector3<f64>) -> f64 {
    let difference = entity_center(entity) - holder_center;
    difference.length()
}

/// Checks whether a leash holder is still present and usable in its world.
#[must_use]
pub fn is_live_holder(holder: &dyn EntityBase) -> bool {
    let entity = holder.get_entity();
    if holder.is_spectator()
        || entity.is_removed()
        || !entity.is_alive()
        || holder
            .get_living_entity()
            .is_some_and(|living| !living.is_alive())
    {
        return false;
    }

    let Some(player) = holder.get_player() else {
        return true;
    };

    entity
        .world
        .load()
        .players
        .load()
        .iter()
        .any(|current| std::ptr::eq(current.as_ref(), player))
}

/// Whether the first passenger is a mob, matching `AbstractNautilus` and `ZombieHorse`.
#[must_use]
pub fn has_mob_control_passenger(entity: &Entity) -> bool {
    entity
        .passengers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .first()
        .is_some_and(|passenger| passenger.get_mob().is_some())
}

impl Entity {
    /// Vanilla `Leashable.tickLeash`, including holder settlement and elastic leash physics.
    pub fn tick_leash(&self) {
        self.tick_leash_inner(None);
    }

    pub(crate) fn tick_leash_with_living(&self, living_entity: &LivingEntity) {
        self.tick_leash_inner(Some(living_entity));
    }

    fn tick_leash_inner(&self, living_entity: Option<&LivingEntity>) {
        let holder = {
            let Ok(guard) = self.leashed_to.try_lock() else {
                return;
            };
            guard.clone()
        };

        if let Some(holder) = holder {
            let holder_entity = holder.get_entity();

            // Removed entities do not tick in vanilla; retain Pumpkin's existing cleanup here.
            if self.is_removed() {
                self.unleash();
                return;
            }

            // Leashable.canInteractWithLevel rejects dead living entities before leash physics.
            if living_entity.is_some_and(|living_entity| !living_entity.is_alive()) {
                drop_lead_after_unleashing(self);
                return;
            }

            if !is_live_holder(holder.as_ref()) {
                drop_lead_after_unleashing(self);
                return;
            }

            let entity_world = self.world.load_full();
            let holder_world = holder_entity.world.load_full();
            // Vanilla only measures distance and applies physics when both entities share a level.
            if !Arc::ptr_eq(&entity_world, &holder_world) {
                return;
            }

            let self_pos = self.pos.load();
            let holder_pos = holder_entity.pos.load();
            let distance = leash_distance(self, holder_entity);

            if distance > self.leash_snap_distance() {
                if self.unleash_if_leashed() {
                    let lead_item = pumpkin_data::item_stack::ItemStack::new(
                        1,
                        &pumpkin_data::item::Item::LEAD,
                    );
                    self.world
                        .load_full()
                        .drop_stack(&self.block_pos.load(), lead_item);
                }
            } else {
                let elastic_distance = self.leash_elastic_distance();
                let elastic_threshold = elastic_distance
                    - f64::from(holder_entity.entity_dimension.load().width)
                    - f64::from(self.entity_dimension.load().width);
                if distance > elastic_threshold {
                    let pull_strength = (distance - elastic_distance) * 0.11;
                    if pull_strength > 0.0 {
                        let dir = (holder_pos - self_pos).normalize();
                        let current_vel = self.velocity.load();
                        self.velocity.store(current_vel + dir * pull_strength);
                        self.velocity_dirty.store(true, Relaxed);
                    }
                }
            }
        }
    }

    /// Attaches this entity to a leash holder.
    pub fn leash_to(&self, holder: Arc<dyn EntityBase>) {
        let _ = self.leash_to_with_result(holder);
    }

    /// Mirrors `Leashable.setLeashedTo`, reporting why the link could not be committed.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the knot reservation borrows the holder while the pointer is replaced"
    )]
    pub(crate) fn leash_to_with_result(&self, holder: Arc<dyn EntityBase>) -> LeashToResult {
        let holder_entity_id = holder.get_entity().entity_id;
        let world = self.world.load_full();
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

        // PlayerLeashEntityEvent may disconnect or replace this player before commit.
        if holder.get_player().is_some() && !is_live_holder(holder.as_ref()) {
            return LeashToResult::HolderUnavailable;
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

        self.world.load_full().broadcast_to_chunk_editioned(
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
            // HappyGhast and Ghast override the 12-block Leashable default.
            EXTENDED_LEASH_SNAP_DISTANCE
        } else {
            Self::LEASH_SNAP_DISTANCE
        }
    }

    /// Returns the vanilla `Leashable.leashElasticDistance` value for this entity type.
    #[must_use]
    pub(crate) fn leash_elastic_distance(&self) -> f64 {
        if self.entity_type == &EntityType::HAPPY_GHAST || self.entity_type == &EntityType::GHAST {
            EXTENDED_LEASH_ELASTIC_DISTANCE
        } else {
            Self::LEASH_ELASTIC_DISTANCE
        }
    }
}

fn drop_lead_after_unleashing(entity: &Entity) {
    if entity.unleash_if_leashed()
        && entity
            .world
            .load_full()
            .level_info
            .load()
            .game_rules
            .entity_drops
    {
        let lead_item =
            pumpkin_data::item_stack::ItemStack::new(1, &pumpkin_data::item::Item::LEAD);
        entity
            .world
            .load_full()
            .drop_stack(&entity.block_pos.load(), lead_item);
    }
}
