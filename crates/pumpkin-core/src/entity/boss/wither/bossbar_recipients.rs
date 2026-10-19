use super::WitherEntity;
use crate::{
    entity::{EntityBase, player::Player},
    world::{
        World,
        bossbar::{Bossbar, recipient::BossbarRecipient},
    },
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct WitherBossbar {
    pub(super) recipients: Vec<Arc<BossbarRecipient>>,
    actions: Vec<BossbarAction>,
    executing: bool,
}

enum BossbarAction {
    Add(Arc<BossbarRecipient>, Bossbar),
    Health(Arc<BossbarRecipient>, f32),
    Remove(Arc<BossbarRecipient>),
}

impl Drop for WitherEntity {
    fn drop(&mut self) {
        // An abandoned batch may drop its last owner while borrowing the effect queue.
        // Never re-enter that queue or invoke packet callbacks during unwinding.
        if std::thread::panicking() {
            return;
        }
        let state = self
            .bossbar_players
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut recipients = std::mem::take(&mut state.recipients);
        recipients.extend(std::mem::take(&mut state.actions).into_iter().map(
            |action| match action {
                BossbarAction::Add(recipient, _)
                | BossbarAction::Health(recipient, _)
                | BossbarAction::Remove(recipient) => recipient,
            },
        ));
        let uuid = self.bossbar_uuid;
        let remove = move || {
            for recipient in recipients {
                if recipient.retire()
                    && let Some(player) = recipient.player()
                {
                    player.remove_bossbar(uuid);
                }
            }
        };
        if let Err(remove) = crate::world::guard_effects::defer(remove) {
            remove();
        }
    }
}

impl WitherEntity {
    // Like the fight dispatcher, re-entry appends effects after the active publication.
    fn with_bossbar<R>(
        &self,
        world: &Arc<World>,
        update: impl FnOnce(&mut WitherBossbar) -> R,
    ) -> R {
        let (result, actions) = {
            let mut state = self
                .bossbar_players
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = update(&mut state);
            let actions = if state.executing || state.actions.is_empty() {
                Vec::new()
            } else {
                state.executing = true;
                std::mem::take(&mut state.actions)
            };
            (result, actions)
        };
        if !actions.is_empty() {
            if let Some(owner) = self.self_reference.upgrade() {
                let request = WitherDispatch {
                    owner,
                    world: world.clone(),
                    actions: Some(actions),
                };
                if let Err(dispatch) = crate::world::guard_effects::defer(move || request.run()) {
                    dispatch();
                }
            } else {
                self.execute_bossbar_actions(actions, world);
            }
        }
        result
    }

    fn execute_bossbar_actions(&self, mut actions: Vec<BossbarAction>, world: &Arc<World>) {
        while !actions.is_empty() {
            for action in actions {
                match action {
                    BossbarAction::Add(recipient, bar) => {
                        self.send_bossbar(world, &recipient, |p| {
                            if recipient.publish() {
                                p.send_bossbar(&bar);
                            }
                        });
                    }
                    BossbarAction::Health(recipient, progress) => {
                        self.send_bossbar(world, &recipient, |p| {
                            if recipient.is_published() {
                                p.update_bossbar_health(&self.bossbar_uuid, progress);
                            }
                        });
                    }
                    BossbarAction::Remove(recipient) => {
                        if recipient.retire()
                            && let Some(player) = recipient.player()
                        {
                            player.remove_bossbar(self.bossbar_uuid);
                        }
                    }
                }
            }
            let mut state = self
                .bossbar_players
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            actions = std::mem::take(&mut state.actions);
            state.executing = !actions.is_empty();
        }
    }

    fn bossbar_recipient(
        &self,
        world: &Arc<World>,
        recipient: &Arc<BossbarRecipient>,
    ) -> Option<Arc<Player>> {
        self.with_bossbar(world, |state| {
            let index = state
                .recipients
                .iter()
                .position(|r| Arc::ptr_eq(r, recipient))?;
            let player = recipient.player();
            let entity = self.get_entity();
            if entity.is_alive()
                && self.mob_entity.living_entity.health.load() > 0.0
                && Arc::ptr_eq(&entity.world.load_full(), world)
                && player
                    .as_ref()
                    .is_some_and(|p| BossbarRecipient::is_current(p, world))
            {
                return player;
            }
            state.recipients.remove(index);
            state.actions.push(BossbarAction::Remove(recipient.clone()));
            None
        })
    }

    fn send_bossbar(
        &self,
        world: &Arc<World>,
        recipient: &Arc<BossbarRecipient>,
        send: impl FnOnce(&Player),
    ) {
        if let Some(player) = self.bossbar_recipient(world, recipient) {
            send(&player);
            self.bossbar_recipient(world, recipient);
        }
    }

    pub(super) fn update_bossbar(&self, world: &Arc<World>, progress: f32) {
        let pos = self.get_entity().pos.load();
        let current: Vec<_> = world
            .players
            .load()
            .iter()
            .filter(|player| {
                BossbarRecipient::is_current(player, world)
                    && (player.living_entity.entity.pos.load() - pos).length_squared() < 50.0 * 50.0
            })
            .cloned()
            .collect();
        let mut bar = self.make_bossbar();
        bar.health = progress;
        self.with_bossbar(world, |state| {
            for player in &current {
                if !state.recipients.iter().any(|r| r.matches(player)) {
                    let recipient = BossbarRecipient::new(player);
                    state
                        .actions
                        .push(BossbarAction::Add(recipient.clone(), bar.clone()));
                    state.recipients.push(recipient);
                }
            }
            state.recipients.retain(|recipient| {
                if current.iter().any(|player| recipient.matches(player)) {
                    return true;
                }
                state.actions.push(BossbarAction::Remove(recipient.clone()));
                false
            });
            for recipient in &state.recipients {
                state
                    .actions
                    .push(BossbarAction::Health(recipient.clone(), progress));
            }
        });
    }

    // ServerBossEvent.removeAllPlayers must not rediscover a departed viewer by UUID.
    pub(super) fn remove_all_bossbar(&self, world: &Arc<World>) {
        self.with_bossbar(world, |state| {
            for recipient in state.recipients.drain(..) {
                state.actions.push(BossbarAction::Remove(recipient));
            }
        });
    }
}

struct WitherDispatch {
    owner: Arc<WitherEntity>,
    world: Arc<World>,
    actions: Option<Vec<BossbarAction>>,
}

impl WitherDispatch {
    fn run(mut self) {
        self.owner
            .execute_bossbar_actions(self.actions.take().unwrap_or_default(), &self.world);
    }
}

impl Drop for WitherDispatch {
    fn drop(&mut self) {
        if let Some(mut actions) = self.actions.take() {
            let mut state = self
                .owner
                .bossbar_players
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            actions.append(&mut state.actions);
            state.actions = actions;
            state.executing = false;
        }
    }
}
