use crate::entity::{
    EntityBase,
    ai::{goal::Controls, goal::Goal, target_predicate::TargetPredicate},
    mob::Mob,
    player::Player,
};
use pumpkin_data::{effect::StatusEffect, potion::Effect};
use rand::RngExt;
use std::sync::Arc;

#[cfg(test)]
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

#[cfg(test)]
thread_local! {
    static DEFAULT_REFRESH_ROLL: std::cell::Cell<Option<std::sync::Arc<AtomicU32>>> =
        const { std::cell::Cell::new(None) };
}

const SWIM_WITH_PLAYER_RANGE: f64 = 10.0;
const SWIM_WITH_PLAYER_CONTINUE_DISTANCE_SQUARED: f64 = 256.0;
const SWIM_WITH_PLAYER_STOP_DISTANCE_SQUARED: f64 = 6.25;
const SWIM_WITH_PLAYER_SPEED: f64 = 4.0;
const DOLPHINS_GRACE_DURATION: i32 = 100;
const DOLPHINS_GRACE_REFRESH_CHANCE: u32 = 6;

pub struct DolphinSwimWithPlayerGoal {
    player: Option<Arc<Player>>,
    #[cfg(test)]
    controlled_refresh_roll: Option<Arc<AtomicU32>>,
}

#[cfg_attr(
    not(test),
    expect(
        clippy::derivable_impls,
        reason = "Test builds consume a scoped deterministic refresh roll."
    )
)]
impl Default for DolphinSwimWithPlayerGoal {
    fn default() -> Self {
        Self {
            player: None,
            #[cfg(test)]
            controlled_refresh_roll: DEFAULT_REFRESH_ROLL.with(std::cell::Cell::take),
        }
    }
}

impl DolphinSwimWithPlayerGoal {
    fn current_target_is_player(mob: &dyn Mob, player: &Player) -> bool {
        mob.get_mob_entity().get_target().is_some_and(|target| {
            target
                .get_player()
                .is_some_and(|target_player| std::ptr::eq(target_player, player))
        })
    }

    fn is_live_swimming_player(
        mob: &dyn Mob,
        player: &Arc<Player>,
        max_distance_squared: f64,
        include_distance_boundary: bool,
    ) -> bool {
        let dolphin = mob.get_entity();
        let player_entity = player.get_entity();
        let world = dolphin.world.load_full();
        let player_world = player_entity.world.load_full();

        Arc::ptr_eq(&world, &player_world)
            && world
                .players
                .load_full()
                .iter()
                .any(|active| Arc::ptr_eq(active, player))
            && player.living_entity.is_part_of_game()
            && player.is_swimming()
            && {
                let distance_squared = dolphin
                    .pos
                    .load()
                    .squared_distance_to_vec(&player_entity.pos.load());
                if include_distance_boundary {
                    distance_squared <= max_distance_squared
                } else {
                    distance_squared < max_distance_squared
                }
            }
    }

    fn add_dolphins_grace(
        mob: &dyn Mob,
        player: &Arc<Player>,
        max_distance_squared: f64,
        include_distance_boundary: bool,
    ) {
        let effect = Effect {
            effect_type: &StatusEffect::DOLPHINS_GRACE,
            duration: DOLPHINS_GRACE_DURATION,
            amplifier: 0,
            ambient: false,
            show_particles: true,
            show_icon: true,
            blend: false,
        };
        player.living_entity.add_effect_if(effect, || {
            Self::is_live_swimming_player(
                mob,
                player,
                max_distance_squared,
                include_distance_boundary,
            )
        });
    }

    #[cfg(test)]
    pub(super) fn with_default_refresh_roll<T>(
        roll: Arc<AtomicU32>,
        construct: impl FnOnce() -> T,
    ) -> T {
        DEFAULT_REFRESH_ROLL.with(|slot| slot.set(Some(roll)));
        let result = construct();
        DEFAULT_REFRESH_ROLL.with(|slot| slot.set(None));
        result
    }

    #[cfg(test)]
    pub(super) fn with_controlled_refresh_roll(initial_roll: u32) -> (Self, Arc<AtomicU32>) {
        let roll = Arc::new(AtomicU32::new(initial_roll));
        (
            Self {
                player: None,
                controlled_refresh_roll: Some(roll.clone()),
            },
            roll,
        )
    }
}

impl Goal for DolphinSwimWithPlayerGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let dolphin = mob.get_entity();
        let world = dolphin.world.load_full();
        let targeting = TargetPredicate::create_non_attackable()
            .set_base_max_distance(SWIM_WITH_PLAYER_RANGE)
            .ignore_visibility();

        self.player = world.get_nearest_player(dolphin.pos.load(), SWIM_WITH_PLAYER_RANGE, |p| {
            targeting.test(&world, Some(mob), p.as_ref())
        });

        self.player.as_ref().is_some_and(|player| {
            player.is_swimming() && !Self::current_target_is_player(mob, player)
        })
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.player.as_ref().is_some_and(|player| {
            Self::is_live_swimming_player(
                mob,
                player,
                SWIM_WITH_PLAYER_CONTINUE_DISTANCE_SQUARED,
                false,
            )
        })
    }

    fn start(&mut self, mob: &dyn Mob) {
        if let Some(player) = &self.player {
            // DolphinSwimWithPlayerGoal.start grants after the inclusive 10-block acquisition.
            Self::add_dolphins_grace(mob, player, SWIM_WITH_PLAYER_RANGE.powi(2), true);
        }
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.player = None;
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(player) = self.player.clone() else {
            return;
        };
        if !Self::is_live_swimming_player(
            mob,
            &player,
            SWIM_WITH_PLAYER_CONTINUE_DISTANCE_SQUARED,
            false,
        ) {
            return;
        }

        let player_entity = player.get_entity();
        let player_pos = player_entity.pos.load();
        mob.get_mob_entity()
            .look_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .look_at_with_range(
                player_pos.x,
                player_entity.get_eye_y(),
                player_pos.z,
                mob.get_max_head_rotation() + 20.0,
                mob.get_max_look_pitch_change(),
            );

        {
            let mut navigator = mob
                .get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if mob
                .get_entity()
                .pos
                .load()
                .squared_distance_to_vec(&player_pos)
                < SWIM_WITH_PLAYER_STOP_DISTANCE_SQUARED
            {
                navigator.stop();
            } else {
                navigator.move_to_entity(
                    &player.living_entity,
                    SWIM_WITH_PLAYER_SPEED,
                    mob.get_mob_entity(),
                );
            }
        }

        let refresh_roll = {
            #[cfg(test)]
            {
                self.controlled_refresh_roll.as_ref().map_or_else(
                    || {
                        mob.get_random()
                            .random_range(0..DOLPHINS_GRACE_REFRESH_CHANCE)
                    },
                    |roll| roll.load(Relaxed),
                )
            }
            #[cfg(not(test))]
            {
                mob.get_random()
                    .random_range(0..DOLPHINS_GRACE_REFRESH_CHANCE)
            }
        };
        if player.is_swimming() && refresh_roll == 0 {
            Self::add_dolphins_grace(
                mob,
                &player,
                SWIM_WITH_PLAYER_CONTINUE_DISTANCE_SQUARED,
                false,
            );
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}
