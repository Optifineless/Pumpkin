use super::{
    DragonFight, FightAction, TIME_BETWEEN_CRYSTAL_SCANS, TIME_BETWEEN_PLAYER_SCANS, World,
};
use crate::entity::{Entity, RemovalReason};
use pumpkin_data::entity::EntityType;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use uuid::Uuid;

impl DragonFight {
    /// Updates fight state, then runs recorded world effects after releasing both guards.
    /// The update must only read the world or record effects; it must not invoke callbacks.
    /// Pass this world's fight mutex; use this entry point for fight mutations.
    /// Re-entry appends effects for the dispatcher to run after the current world operation.
    pub fn with_fight<R>(
        fight_mutex: &Mutex<Self>,
        world: &Arc<World>,
        update: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let fight = fight_mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (result, actions) = Self::update_locked(fight, world, update);
        Self::execute_actions(actions, world);
        result
    }

    fn update_locked<R>(
        mut fight: MutexGuard<'_, Self>,
        world: &Arc<World>,
        update: impl FnOnce(&mut Self) -> R,
    ) -> (R, Vec<FightAction>) {
        // Lock fight then queue; producers release the queue before trying the fight.
        // Holding both here serializes onCrystalDestroyed with the End transition.
        let mut notifications = world
            .pending_destroyed_crystals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for uuid in notifications.drain(..) {
            fight.on_crystal_destroyed(world, uuid);
        }
        let result = update(&mut fight);
        let actions = if fight.executing_actions || fight.actions.is_empty() {
            Vec::new()
        } else {
            fight.executing_actions = true;
            std::mem::take(&mut fight.actions)
        };
        (result, actions)
    }

    fn execute_actions(mut actions: Vec<FightAction>, world: &Arc<World>) {
        #[cfg(test)]
        if !actions.is_empty() {
            super::callback_tests::observe_before_actions();
        }
        while !actions.is_empty() {
            for action in actions {
                action.execute(world);
            }
            let Some(fight_mutex) = &world.dragon_fight else {
                return;
            };
            let fight = fight_mutex
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Re-entry records effects for this dispatcher, after the current world operation.
            let ((), next) = Self::update_locked(fight, world, |fight| {
                fight.executing_actions = false;
            });
            actions = next;
        }
    }

    /// Delivers EndCrystal.onDestroyedBy without blocking on a re-entered fight guard.
    pub(crate) fn notify_crystal_destroyed(world: &Arc<World>, uuid: Uuid) {
        let Some(fight_mutex) = &world.dragon_fight else {
            return;
        };
        world
            .pending_destroyed_crystals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(uuid);
        let fight = match fight_mutex.try_lock() {
            Ok(fight) => fight,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        let ((), actions) = Self::update_locked(fight, world, |_| ());
        Self::execute_actions(actions, world);
    }

    pub fn tick(fight_mutex: &Mutex<Self>, world: &Arc<World>) {
        let ready = Self::with_fight(fight_mutex, world, |fight| {
            fight.ticks_since_last_player_scan += 1;
            if fight.ticks_since_last_player_scan >= TIME_BETWEEN_PLAYER_SCANS {
                fight.update_players(world);
                fight.ticks_since_last_player_scan = 0;
            }
            !fight.bossbar_players.is_empty() && fight.is_arena_loaded(world)
        });
        if !ready {
            return;
        }

        Self::with_fight(fight_mutex, world, |fight| {
            if fight.needs_state_scanning {
                fight.scan_state(world);
                fight.needs_state_scanning = false;
            }
        });

        // EnderDragonFight.tick: finish scan callbacks before advancing the animation.
        let respawning = Self::with_fight(fight_mutex, world, |fight| {
            if let Some(stage) = fight.respawn_stage {
                let action = fight.tick_respawn_stage(world, stage);
                if fight.respawn_stage.is_some() {
                    fight.respawn_time += 1;
                }
                if let Some(action) = action {
                    fight.actions.push(FightAction::Respawn {
                        action,
                        generation: fight.respawn_generation,
                    });
                }
                return true;
            }

            if !fight.dragon_killed {
                fight.ticks_since_dragon_seen += 1;
                if fight.dragon_uuid.is_none()
                    || fight.ticks_since_dragon_seen >= super::MAX_TICKS_BEFORE_DRAGON_RESPAWN
                {
                    fight.find_or_create_dragon(world);
                    fight.ticks_since_dragon_seen = 0;
                }
            }
            false
        });
        if respawning {
            return;
        }
        Self::with_fight(fight_mutex, world, |fight| {
            if !fight.dragon_killed {
                fight.ticks_since_crystals_scanned += 1;
                if fight.ticks_since_crystals_scanned >= TIME_BETWEEN_CRYSTAL_SCANS {
                    fight.update_crystal_count(world);
                    fight.ticks_since_crystals_scanned = 0;
                }
            }
        });
    }
}

impl World {
    /// Marks removal and returns whether entity cleanup should run.
    #[must_use]
    pub(in crate::world) fn begin_entity_removal(&self, entity: &Entity) -> bool {
        // Entity.setRemoved marks before callbacks; serialize EndCrystal.hurtServer with completion.
        let _notifications = (self.dragon_fight.is_some()
            && entity.entity_type == &EntityType::END_CRYSTAL)
            .then(|| {
                #[cfg(test)]
                super::removal_lock_tests::observe_waiting(&self.pending_destroyed_crystals);
                self.pending_destroyed_crystals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            });
        let needs_cleanup = entity
            .removal_reason
            .compare_exchange(None, Some(RemovalReason::Discarded))
            .is_ok();
        #[cfg(test)]
        super::removal_lock_tests::observe_marked();
        needs_cleanup
    }
}
