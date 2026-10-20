//! Vanilla 26.3 `RunAroundLikeCrazyGoal` and `AbstractHorse.tameWithName/makeMad`.
use std::sync::{Arc, Weak};

use super::horse::{FLAG_EATING, FLAG_STANDING, FLAG_TAME, HorseEntity};
use crate::{
    entity::{
        EntityBase,
        ai::{
            goal::{Controls, Goal},
            pathfinder::NavigatorGoal,
            util::default_random_pos,
        },
        mob::Mob,
    },
    plugin::api::events::entity::entity_tame::EntityTameEvent,
    world::World,
};
use pumpkin_data::{advancement::Advancement, entity_status::EntityStatus, sound::Sound};
use pumpkin_util::math::vector3::Vector3;
use rand::RngExt;

const SPEED_MODIFIER: f64 = 1.2;
const TAMING_INTERVAL: i32 = 50;
const TEMPER_INCREMENT: i32 = 5;
const STAND_TICKS: u8 = 20;

pub fn goal_selector_age(mob: &crate::entity::mob::MobEntity) -> i32 {
    use std::sync::atomic::Ordering::Relaxed;
    let entity = &mob.living_entity.entity;
    // Horse's Entity.age is also its breeding age and returns to zero before AI.
    // The existing lifetime counter gives GoalSelector its vanilla alternating cadence.
    if entity.entity_type == &pumpkin_data::entity::EntityType::HORSE
        && entity
            .world
            .load()
            .server
            .upgrade()
            .is_none_or(|server| server.advanced_config.horse_riding_taming.enabled)
    {
        mob.ticks_lived.load(Relaxed)
    } else {
        entity.age.load(Relaxed)
    }
}

pub(super) struct RunAroundLikeCrazyGoal {
    horse: Weak<HorseEntity>,
    target: Vector3<f64>,
}

impl RunAroundLikeCrazyGoal {
    pub const fn new(horse: Weak<HorseEntity>) -> Self {
        Self {
            horse,
            target: Vector3::new(0.0, 0.0, 0.0),
        }
    }
}

impl Goal for RunAroundLikeCrazyGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let Some(horse) = self.horse.upgrade() else {
            return false;
        };
        // Ordinary Horse.isMobControlled is always false; no generic riding API is needed.
        if !horse.riding_taming_enabled() || horse.is_tame() || !horse.get_entity().has_passengers()
        {
            return false;
        }
        let Some(target) = default_random_pos::get_pos(mob, 5, 4) else {
            return false;
        };
        self.target = target;
        true
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.horse.upgrade().is_some_and(|horse| {
            horse.riding_taming_enabled()
                && !horse.is_tame()
                && horse.get_entity().has_passengers()
                && !mob.is_navigator_idle()
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_progress(NavigatorGoal::new(
                mob.get_entity().pos.load(),
                self.target,
                SPEED_MODIFIER,
            ));
    }

    fn tick(&mut self, _mob: &dyn Mob) {
        let Some(horse) = self.horse.upgrade() else {
            return;
        };
        if !horse.riding_taming_enabled() || horse.is_tame() {
            return;
        }
        // Default goals run on alternate server ticks. adjustedTickDelay(50) is 25 rolls.
        if horse.taming_roll(self.get_tick_count(TAMING_INTERVAL), false) == 0 {
            horse.ride_attempt();
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

// Only owned identities cross arbitrary code. Drop releases this reservation on every exit/unwind.
struct Attempt {
    horse: Arc<HorseEntity>,
    world: Arc<World>,
    passenger: Arc<dyn EntityBase>,
    token: u64,
    revision: u64,
    riding_epoch: u64,
    horse_life: u64,
    passenger_life: Option<u64>,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        let mut state = self
            .horse
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.pending == Some(self.token) {
            state.pending = None;
        }
    }
}

impl Attempt {
    fn horse_is_current(&self) -> bool {
        let entity = self.horse.get_entity();
        self.horse.get_mob_entity().living_entity.damage_lifecycle() == self.horse_life
            && !entity.is_removed()
            && self.horse.get_mob_entity().living_entity.is_alive()
            && Arc::ptr_eq(&entity.world.load_full(), &self.world)
            && self
                .world
                .get_entity_by_id(entity.entity_id)
                .is_some_and(|current| {
                    std::ptr::eq(current.get_entity(), entity)
                        && current
                            .cast_any()
                            .downcast_ref::<HorseEntity>()
                            .is_some_and(|horse| std::ptr::eq(horse, self.horse.as_ref()))
                })
    }

    fn passenger_is_current(&self) -> bool {
        let entity = self.passenger.get_entity();
        if self
            .passenger
            .get_living_entity()
            .map(crate::entity::living::LivingEntity::damage_lifecycle)
            != self.passenger_life
            || entity.is_removed()
            || !Arc::ptr_eq(&entity.world.load_full(), &self.world)
            || self
                .passenger
                .get_living_entity()
                .is_some_and(|living| !living.is_alive())
        {
            return false;
        }
        self.passenger.get_player().map_or_else(
            || {
                self.world
                    .get_entity_by_id(entity.entity_id)
                    .is_some_and(|current| Arc::ptr_eq(&current, &self.passenger))
            },
            |player| {
                !player.client.closed()
                    && self
                        .world
                        .get_player_by_uuid(entity.entity_uuid)
                        .is_some_and(|current| std::ptr::eq(current.as_ref(), player))
            },
        )
    }

    fn still_riding(&self) -> bool {
        self.state_is_current()
            && self.horse_is_current()
            && self.passenger_is_current()
            && self
                .horse
                .get_entity()
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .first()
                .is_some_and(|first| Arc::ptr_eq(first, &self.passenger))
            && self
                .passenger
                .get_entity()
                .get_vehicle()
                .is_some_and(|vehicle| std::ptr::eq(vehicle.get_entity(), self.horse.get_entity()))
    }

    fn state_is_current(&self) -> bool {
        let state = self
            .horse
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending == Some(self.token)
            && state.revision == self.revision
            && state.riding_epoch == self.riding_epoch
    }

    fn tame_with_name(&mut self) {
        let Some(player) = self.passenger.get_player() else {
            return;
        };
        let Some(owner) = self.world.get_player_by_uuid(player.gameprofile.id) else {
            return;
        };
        let Some(server) = self.world.server.upgrade() else {
            return;
        };
        if !std::ptr::eq(owner.as_ref(), player) || !self.still_riding() {
            return;
        }
        let mut event = EntityTameEvent::new(self.horse.get_entity().entity_id, owner.clone());
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled || !self.still_riding() {
            return;
        }
        {
            let mut state = self
                .horse
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending != Some(self.token)
                || state.revision != self.revision
                || state.riding_epoch != self.riding_epoch
                || self.horse.get_entity().is_removed()
            {
                return;
            }
            // The original admitted session owns the horse, regardless of a plugin's payload edits.
            state.owner = Some(owner.gameprofile.id);
            state.flags |= FLAG_TAME;
            state.revision = state.revision.wrapping_add(1);
            self.revision = state.revision;
        };
        if !self.still_riding() || !self.state_is_current() {
            return;
        }
        self.horse.publish_flags();
        owner.trigger_advancement_criterion(Advancement::HUSBANDRY_TAME_AN_ANIMAL, "tamed_animal");
        if self.still_riding() && self.state_is_current() {
            self.world.send_entity_status(
                self.horse.get_entity(),
                EntityStatus::TamingSucceeded,
                None,
            );
        }
    }

    fn buck(&mut self) {
        let passengers = self
            .horse
            .get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for passenger in passengers {
            if !self.horse_is_current() || !self.passenger_is_current() || !self.state_is_current()
            {
                return;
            }
            self.horse
                .get_entity()
                .remove_passenger(passenger.get_entity().entity_id);
            let epoch = self
                .horse
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .riding_epoch;
            // Only this accepted removal may advance the observation. Callback ABA invalidates it.
            if epoch != self.riding_epoch.wrapping_add(1) {
                return;
            }
            self.riding_epoch = epoch;
        }
        // The public removal path owns cancellation; a veto never authorizes forced unlinking.
        if !self.horse_is_current()
            || !self.passenger_is_current()
            || !self.state_is_current()
            || self.horse.get_entity().has_passengers()
            || self.passenger.get_entity().has_vehicle()
        {
            return;
        }
        let make_mad = {
            let mut state = self
                .horse
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending != Some(self.token)
                || state.revision != self.revision
                || state.riding_epoch != self.riding_epoch
                || self.horse.get_entity().is_removed()
            {
                return;
            }
            if state.flags & FLAG_STANDING == 0 {
                state.flags = (state.flags & !FLAG_EATING) | FLAG_STANDING;
                state.standing_ticks = STAND_TICKS;
                true
            } else {
                false
            }
        };
        self.horse.publish_flags();
        if make_mad {
            self.horse.get_entity().play_sound(Sound::EntityHorseAngry);
        }
        if self.horse_is_current() && self.passenger_is_current() && self.state_is_current() {
            self.world.send_entity_status(
                self.horse.get_entity(),
                EntityStatus::TamingFailed,
                None,
            );
        }
    }
}

impl HorseEntity {
    pub(super) fn riding_taming_enabled(&self) -> bool {
        self.get_entity()
            .world
            .load()
            .server
            .upgrade()
            .is_none_or(|server| server.advanced_config.horse_riding_taming.enabled)
    }

    fn ride_attempt(self: &Arc<Self>) {
        let Some(passenger) = self
            .get_entity()
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .cloned()
        else {
            return;
        };
        let horse_life = self.get_mob_entity().living_entity.damage_lifecycle();
        let passenger_life = passenger
            .get_living_entity()
            .map(crate::entity::living::LivingEntity::damage_lifecycle);
        let mut attempt = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending.is_some()
                || state.flags & FLAG_TAME != 0
                || self.get_entity().is_removed()
            {
                return;
            }
            state.next_attempt = state.next_attempt.wrapping_add(1);
            state.pending = Some(state.next_attempt);
            Attempt {
                horse: self.clone(),
                world: self.get_entity().world.load_full(),
                passenger,
                token: state.next_attempt,
                revision: state.revision,
                riding_epoch: state.riding_epoch,
                horse_life,
                passenger_life,
            }
        };
        if !attempt.still_riding() {
            return;
        }
        if attempt.passenger.get_player().is_some() {
            let temper = self.get_temper();
            let max = self.get_max_temper();
            if max > 0 && self.taming_roll(max, true) < temper {
                attempt.tame_with_name();
                return;
            }
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pending != Some(attempt.token)
                || state.revision != attempt.revision
                || state.riding_epoch != attempt.riding_epoch
                || self.get_entity().is_removed()
            {
                return;
            }
            state.temper = state.temper.saturating_add(TEMPER_INCREMENT).clamp(0, max);
            state.revision = state.revision.wrapping_add(1);
            attempt.revision = state.revision;
        }
        attempt.buck();
    }

    fn taming_roll(&self, upper: i32, temper: bool) -> i32 {
        #[cfg(test)]
        if let Some(value) = self.taming_random.roll(upper, temper) {
            return value;
        }
        #[cfg(not(test))]
        let _ = temper;
        self.get_random().random_range(0..upper)
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct TestRandom {
    pub script: std::sync::Mutex<Option<(i32, i32)>>,
    pub rolls: std::sync::Mutex<Vec<(i32, bool)>>,
}

#[cfg(test)]
impl TestRandom {
    fn roll(&self, upper: i32, temper: bool) -> Option<i32> {
        let script = *self
            .script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        script.map(|(attempt_roll, temper_roll)| {
            self.rolls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((upper, temper));
            if temper { temper_roll } else { attempt_roll }
        })
    }
}
