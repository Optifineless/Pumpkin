//! Entity.applyMovementEmissionAndPlaySound, after committed travel and fall callbacks.

use super::{Entity, EntityBase, living::LivingEntity};
use crate::world::{World, game_events::GameEventContext};
use pumpkin_data::{
    Block, BlockStateId,
    game_event::GameEvent,
    tag::{self, Taggable},
};
use pumpkin_util::{
    math::vector3::Vector3,
    sync::{LockClass, Mutex},
};
use std::sync::{Arc, atomic::Ordering::Relaxed};

mod support;
#[cfg(test)]
mod tests;

pub(super) enum MovementEmissionOwner {}
impl LockClass for MovementEmissionOwner {
    const NAME: &'static str = "movement emission";
    const LEVEL: u8 = 30;
}

pub(super) type MovementEmissionState = Mutex<Distances, MovementEmissionOwner>;

#[derive(Clone, Copy)]
pub(super) struct Distances {
    move_dist: f32,
    fly_dist: f32,
    next_step: f32,
    revision: u64,
    sample: Option<(usize, Option<u64>, Vector3<f64>)>,
}

impl Default for Distances {
    fn default() -> Self {
        Self {
            move_dist: 0.0,
            fly_dist: 0.0,
            next_step: 1.0,
            revision: 0,
            sample: None,
        }
    }
}

#[derive(Clone, Copy)]
enum SupportPosition {
    Cached,
    Supplied(Option<pumpkin_util::math::position::BlockPos>),
}

/// An unlocked observation of the original admitted movement and life.
pub struct MovementSample<'a> {
    living: Option<&'a LivingEntity>,
    player: Option<&'a super::player::Player>,
    support: SupportPosition,
    entity: usize,
    entity_id: i32,
    uuid: uuid::Uuid,
    client: Option<usize>,
    world: Arc<World>,
    from: Vector3<f64>,
    life: Option<u64>,
    teleport: Option<i32>,
    revision: u64,
}

impl Entity {
    pub(crate) fn begin_movement_emission<'a>(
        &self,
        caller: &'a dyn EntityBase,
    ) -> Option<MovementSample<'a>> {
        let world = self.world.load_full();
        if !world
            .server
            .upgrade()?
            .advanced_config
            .movement_game_events
            .enabled
        {
            return None;
        }
        let from = self.pos.load();
        let living = caller.get_living_entity();
        let player = caller.get_player();
        let life = living.map(LivingEntity::damage_lifecycle);
        let teleport = player.map(|player| player.teleport_id_count.load(Relaxed));
        let client = player.map(|player| Arc::as_ptr(&player.client) as usize);
        let mut sample = MovementSample {
            living,
            player,
            support: SupportPosition::Cached,
            entity: std::ptr::from_ref(self) as usize,
            entity_id: self.entity_id,
            uuid: self.entity_uuid,
            client,
            world,
            from,
            life,
            teleport,
            revision: 0,
        };
        if !sample.is_current(caller) || !finite(from) {
            return None;
        }
        let key = (Arc::as_ptr(&sample.world) as usize, life, from);
        let mut state = self
            .movement_emission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.sample.is_some_and(|previous| previous != key) {
            let revision = state.revision.wrapping_add(1);
            *state = Distances {
                revision,
                ..Distances::default()
            };
        }
        state.sample = Some(key);
        sample.revision = state.revision;
        drop(state);
        Some(sample)
    }

    pub(crate) fn reset_movement_emission(&self) {
        let mut state = self
            .movement_emission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let revision = state.revision.wrapping_add(1);
        *state = Distances {
            revision,
            ..Distances::default()
        };
    }
}

impl MovementSample<'_> {
    pub(crate) const fn with_support(
        mut self,
        support: Option<pumpkin_util::math::position::BlockPos>,
    ) -> Self {
        self.support = SupportPosition::Supplied(support);
        self
    }
    pub(crate) fn is_current(&self, caller: &dyn EntityBase) -> bool {
        let entity = caller.get_entity();
        if std::ptr::from_ref(entity) as usize != self.entity
            || entity.entity_id != self.entity_id
            || entity.entity_uuid != self.uuid
            || entity.is_removed()
            || !Arc::ptr_eq(&self.world, &entity.world.load_full())
            || caller
                .get_living_entity()
                .map(LivingEntity::damage_lifecycle)
                != self.life
            || caller
                .get_living_entity()
                .is_some_and(|living| living.dead.load(Relaxed) || living.health.load() <= 0.0)
        {
            return false;
        }
        let Some(tracked) = self
            .world
            .entity_tracker
            .get_tracked_entity(entity.entity_id)
        else {
            return false;
        };
        let current = tracked.entity.get_entity();
        if !std::ptr::eq(current, entity) || current.entity_uuid != entity.entity_uuid {
            return false;
        }
        caller.get_player().is_none_or(|player| {
            Some(Arc::as_ptr(&player.client) as usize) == self.client
                && !player.client.closed()
                && player.has_client_loaded()
                && self
                    .world
                    .server
                    .upgrade()
                    .is_some_and(|server| server.can_track_player(player))
                && match player.client.as_ref() {
                    crate::net::ClientPlatform::Java(client) => client
                        .player
                        .load()
                        .as_ref()
                        .as_ref()
                        .is_some_and(|current| std::ptr::eq(current.as_ref(), player)),
                    crate::net::ClientPlatform::Bedrock(client) => client
                        .player
                        .load()
                        .as_ref()
                        .as_ref()
                        .is_some_and(|current| std::ptr::eq(current.as_ref(), player)),
                }
                && Some(player.teleport_id_count.load(Relaxed)) == self.teleport
                && player
                    .awaiting_teleport
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_none()
        })
    }

    pub(crate) fn finish(
        self,
        caller: &dyn EntityBase,
        from: Vector3<f64>,
        position: Vector3<f64>,
        movement: Vector3<f64>,
        on_ground: bool,
    ) {
        let entity = caller.get_entity();
        if !finite(position)
            || !finite(movement)
            || from != self.from
            || entity.pos.load() != position
            || !self.is_current(caller)
            || movement == Vector3::default()
            || entity.no_physics.load(Relaxed)
            || entity.has_vehicle()
        {
            return;
        }
        // Virtual methods and terrain access run before the scalar counter owner.
        let policy = movement_policy(caller);
        let support = match self.support {
            SupportPosition::Cached => entity.supporting_block_pos.load(),
            SupportPosition::Supplied(position) => position,
        };
        let blocks = support::Blocks::capture(&self.world, entity, support);
        let flapping = is_flapping(caller);
        let crouching = entity.pose.load() == pumpkin_data::entity::EntityPose::Crouching;
        let swimming = entity.swimming.load(Relaxed);
        let water = entity.touching_water.load(Relaxed);
        let Some(blocks) = blocks else {
            return;
        };
        let rails = caller
            .cast_any()
            .is::<super::vehicle::minecart::MinecartEntity>()
            && Block::from_state_id(blocks.support.state).has_tag(&tag::Block::MINECRAFT_RAILS);
        let eligible = |state| {
            !pumpkin_data::BlockState::from_id(state).is_air()
                && (on_ground || climbable(state) || crouching && movement.y == 0.0 || rails)
                && !swimming
        };
        let climbing = climbable(blocks.support.state);
        let support_air = pumpkin_data::BlockState::from_id(blocks.support.state).is_air();
        let support_effect = eligible(blocks.support.state);
        let effect_effect = eligible(blocks.effect.state);
        if !blocks.is_current(&self.world)
            || !self.is_current(caller)
            || entity.pos.load() != position
        {
            return;
        }
        let event = self.record(
            entity,
            position,
            movement,
            &blocks,
            Effects {
                climbing,
                support_air,
                effect: effect_effect,
                support: support_effect,
                water,
                flapping,
            },
            policy,
        );
        if let Some((event, affected_state)) = event
            && self.is_current(caller)
            && entity.pos.load() == position
            && blocks.is_current(&self.world)
        {
            self.world.emit_game_event_with_context(
                event.name(),
                position,
                GameEventContext {
                    source: Some(entity),
                    affected_state,
                },
            );
        }
    }
    fn record(
        &self,
        entity: &Entity,
        position: Vector3<f64>,
        movement: Vector3<f64>,
        blocks: &support::Blocks,
        effects: Effects,
        policy: MovementEmission,
    ) -> Option<(GameEvent, Option<BlockStateId>)> {
        let mut state = entity
            .movement_emission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.revision != self.revision
            || entity.removal_reason.load().is_some()
            || entity.pos.load() != position
            || !std::ptr::eq(entity.world.load().as_ref(), self.world.as_ref())
            || self.living.map(LivingEntity::damage_lifecycle) != self.life
            || self
                .living
                .is_some_and(|living| living.dead.load(Relaxed) || living.health.load() <= 0.0)
            || self
                .player
                .is_some_and(|player| Some(player.teleport_id_count.load(Relaxed)) != self.teleport)
        {
            return None;
        }
        state.sample = Some((Arc::as_ptr(&self.world) as usize, self.life, position));
        state.revision = state.revision.wrapping_add(1);
        let event = if policy.emits_anything() {
            apply_movement_emission(&mut state, movement, blocks, effects, policy)
        } else {
            None
        };
        drop(state);
        event
    }
}

#[derive(Clone, Copy)]
enum MovementEmission {
    None,
    Events,
    All,
}
impl MovementEmission {
    const fn emits_anything(self) -> bool {
        !matches!(self, Self::None)
    }
    const fn emits_events(self) -> bool {
        matches!(self, Self::Events | Self::All)
    }
}

fn movement_policy(caller: &dyn EntityBase) -> MovementEmission {
    let entity = caller.get_entity();
    if caller.get_player().is_some_and(|player| {
        let flying = player
            .abilities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .flying;
        flying
            || caller.is_spectator()
            || entity.on_ground.load(Relaxed) && entity.sneaking.load(Relaxed)
    }) {
        return MovementEmission::None;
    }
    // getMovementEmission overrides, including inherited arrow, squid and vehicle variants.
    match entity.entity_type.resource_name {
        "end_crystal" | "experience_orb" | "falling_block" | "item" | "tnt" | "shulker"
        | "arrow" | "spectral_arrow" | "trident" | "fishing_bobber" => MovementEmission::None,
        "bat" | "squid" | "glow_squid" | "breeze" | "endermite" | "guardian" | "elder_guardian"
        | "silverfish" => MovementEmission::Events,
        name if name.ends_with("_boat")
            || name.ends_with("_raft")
            || name.ends_with("minecart") =>
        {
            MovementEmission::Events
        }
        _ => MovementEmission::All,
    }
}

fn climbable(state: BlockStateId) -> bool {
    let block = Block::from_state_id(state);
    block == &Block::POWDER_SNOW || block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE)
}

fn is_flapping(caller: &dyn EntityBase) -> bool {
    if let Some(bat) = caller
        .cast_any()
        .downcast_ref::<super::mob::bat::BatEntity>()
    {
        return bat.is_flapping();
    }
    let entity = caller.get_entity();
    let Some(mob) = caller.get_mob() else {
        return false;
    };
    let ticks = mob.get_mob_entity().ticks_lived.load(Relaxed);
    // These species inherit Entity.onFlap's no-op and already own this tick/ground state.
    match entity.entity_type.resource_name {
        "allay" => !entity.on_ground.load(Relaxed),
        "bee" => !entity.on_ground.load(Relaxed) && ticks % 2 == 0,
        "vex" => ticks % 4 == 0,
        "phantom" => entity.entity_id.wrapping_mul(3).wrapping_add(ticks) % 25 == 0,
        _ => false,
    }
}

const fn finite(position: Vector3<f64>) -> bool {
    position.x.is_finite() && position.y.is_finite() && position.z.is_finite()
}

#[derive(Clone, Copy)]
struct Effects {
    climbing: bool,
    support_air: bool,
    effect: bool,
    support: bool,
    water: bool,
    flapping: bool,
}

// Sounds remain with existing owners; the helper's eligibility is its Java return value.
#[expect(
    clippy::imprecise_flops,
    reason = "Preserve Java Vec3 length evaluation and float rounding at step thresholds"
)]
fn apply_movement_emission(
    state: &mut Distances,
    delta: Vector3<f64>,
    blocks: &support::Blocks,
    effects: Effects,
    policy: MovementEmission,
) -> Option<(GameEvent, Option<BlockStateId>)> {
    let horizontal = (delta.x * delta.x + delta.z * delta.z).sqrt();
    let full = ((delta.x * delta.x + delta.y * delta.y) + delta.z * delta.z).sqrt();
    let move_dist_scale = f64::from(0.6f32);
    let scaled_full = (full * move_dist_scale) as f32;
    state.move_dist += if effects.climbing {
        scaled_full
    } else {
        (horizontal * move_dist_scale) as f32
    };
    state.fly_dist += scaled_full;
    if state.move_dist > state.next_step && !effects.support_air {
        let same = blocks.support.pos == blocks.effect.pos;
        let produced = effects.effect || !same && effects.support;
        if produced || effects.water {
            state.next_step = (state.move_dist as i32).wrapping_add(1) as f32;
        }
        if same && effects.effect {
            Some((GameEvent::Step, Some(blocks.effect.state)))
        } else if !same && effects.support && policy.emits_events() {
            Some((GameEvent::Step, Some(blocks.support.state)))
        } else if !produced && effects.water && policy.emits_events() {
            Some((GameEvent::Swim, None))
        } else {
            None
        }
    } else if effects.support_air && effects.flapping && policy.emits_events() {
        // Supported species inherit Entity's no-op; other flap owners stay there.
        Some((GameEvent::Flap, None))
    } else {
        None
    }
}
