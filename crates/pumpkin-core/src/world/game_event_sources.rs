//! Current, world-local game-event membership, independent of tracking audiences.

use std::sync::{Arc, Weak};

use pumpkin_util::sync::{LockClass, Mutex};
use rustc_hash::FxHashMap;

use super::World;
use crate::entity::player::Player;
use crate::entity::{Entity, EntityBase};

pub(super) enum SourceIndexLock {}

impl LockClass for SourceIndexLock {
    const NAME: &'static str = "game-event source membership";
    const LEVEL: u8 = 20;
}

pub(super) struct SourceMembership {
    entity: Weak<dyn EntityBase>,
    instance: usize,
}

pub(super) struct SourceIndex {
    state: Mutex<SourceIndexState, SourceIndexLock>,
    #[cfg(test)]
    pub probes: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub comparisons: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct SourceIndexState {
    entries: FxHashMap<i32, Arc<SourceMembership>>,
    closed: bool,
}

impl Default for SourceIndex {
    fn default() -> Self {
        Self {
            state: Mutex::new(SourceIndexState::default()),
            #[cfg(test)]
            probes: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            comparisons: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl World {
    pub(super) fn publish_game_event_entities(
        &self,
        members: &[Arc<dyn EntityBase>],
        mut update: impl FnMut(&Arc<Vec<Arc<dyn EntityBase>>>) -> Option<Vec<Arc<dyn EntityBase>>>,
    ) -> bool {
        let additions: Vec<_> = members
            .iter()
            .map(|entity| {
                let base = entity.get_entity();
                (
                    base.entity_id,
                    Arc::new(SourceMembership {
                        entity: Arc::downgrade(entity),
                        instance: std::ptr::from_ref(base) as usize,
                    }),
                )
            })
            .collect();
        loop {
            let previous = self.entities.load_full();
            let Some(next) = update(&previous) else {
                if !Arc::ptr_eq(&previous, &self.entities.load_full()) {
                    continue;
                }
                return false;
            };
            let mut state = self
                .game_event_sources
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = self.entities.compare_and_swap(&previous, Arc::new(next));
            if Arc::ptr_eq(&current, &previous) {
                if !state.closed {
                    state.entries.extend(additions.iter().cloned());
                }
                return true;
            }
        }
    }

    #[expect(
        clippy::rc_buffer,
        reason = "CAS images must match the existing World.players ArcSwap<Vec<Arc<Player>>> storage"
    )]
    pub(crate) fn commit_game_event_player(
        &self,
        player: &Arc<Player>,
        previous: &Arc<Vec<Arc<Player>>>,
        next: Arc<Vec<Arc<Player>>>,
    ) -> bool {
        let entity: Arc<dyn EntityBase> = player.clone();
        let base = entity.get_entity();
        let id = base.entity_id;
        let membership = Arc::new(SourceMembership {
            entity: Arc::downgrade(&entity),
            instance: std::ptr::from_ref(base) as usize,
        });
        let mut state = self
            .game_event_sources
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = self.players.compare_and_swap(previous, next);
        if !Arc::ptr_eq(&current, previous) {
            return false;
        }
        let replaced = if state.closed {
            None
        } else {
            state.entries.insert(id, membership)
        };
        drop(state);
        drop(replaced);
        true
    }

    pub(crate) fn withdraw_game_event_player(&self, player: &Arc<Player>) -> Option<Arc<Player>> {
        let base = player.get_entity();
        let id = base.entity_id;
        let instance = std::ptr::from_ref(base) as usize;
        let mut removed = None;
        let mut state = self
            .game_event_sources
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.players.rcu(|current| {
            let mut next = (**current).clone();
            removed = next
                .iter()
                .position(|current| Arc::ptr_eq(current, player))
                .map(|pos| next.remove(pos));
            next
        });
        let membership = if state
            .entries
            .get(&id)
            .is_some_and(|entry| entry.instance == instance)
        {
            state.entries.remove(&id)
        } else {
            None
        };
        drop(state);
        drop(membership);
        removed
    }

    pub(crate) fn retire_game_event_source(&self, entity: &Entity) {
        let instance = std::ptr::from_ref(entity) as usize;
        let removed = {
            let mut state = self
                .game_event_sources
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state
                .entries
                .get(&entity.entity_id)
                .is_some_and(|entry| entry.instance == instance)
            {
                state.entries.remove(&entity.entity_id)
            } else {
                None
            }
        };
        drop(removed);
    }

    // A dimension transfer does not mark its source removed. Its delayed cleanup
    // must not retire a new admission of the same Arc after a return transfer.
    pub(crate) fn game_event_source_retirement<'a>(
        &'a self,
        entity: &Entity,
    ) -> impl FnOnce() + 'a {
        let id = entity.entity_id;
        let instance = std::ptr::from_ref(entity) as usize;
        let expected = self
            .game_event_source_membership(id)
            .filter(|entry| entry.instance == instance);
        move || {
            let removed = {
                let mut state = self
                    .game_event_sources
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if expected.as_ref().is_some_and(|expected| {
                    state
                        .entries
                        .get(&id)
                        .is_some_and(|current| Arc::ptr_eq(current, expected))
                }) {
                    state.entries.remove(&id)
                } else {
                    None
                }
            };
            drop(removed);
        }
    }

    pub(super) fn game_event_source_membership(&self, id: i32) -> Option<Arc<SourceMembership>> {
        let state = self
            .game_event_sources
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return None;
        }
        #[cfg(test)]
        self.game_event_sources
            .probes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        state.entries.get(&id).cloned()
    }

    pub(super) fn close_game_event_sources(&self) {
        let retired = {
            let mut state = self
                .game_event_sources
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            std::mem::take(&mut state.entries)
        };
        drop(retired);
    }
}

impl SourceMembership {
    pub(super) fn entity(&self) -> Option<Arc<dyn EntityBase>> {
        self.entity.upgrade()
    }
}
