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
                    actions: Some(actions.into()),
                };
                if let Err(dispatch) = crate::world::guard_effects::defer(move || request.run()) {
                    dispatch();
                }
            } else {
                WitherExecution {
                    owner: self,
                    world,
                    actions: actions.into(),
                    completed: false,
                }
                .run();
            }
        }
        result
    }

    fn execute_bossbar_action(&self, action: &BossbarAction, world: &Arc<World>) {
        match action {
            BossbarAction::Add(recipient, bar) => {
                self.send_bossbar(world, recipient, |player| {
                    if !recipient.is_published() {
                        player.send_bossbar(bar);
                        recipient.publish();
                    }
                });
            }
            BossbarAction::Health(recipient, progress) => {
                self.send_bossbar(world, recipient, |player| {
                    if recipient.is_published() {
                        player.update_bossbar_health(&self.bossbar_uuid, *progress);
                    }
                });
            }
            BossbarAction::Remove(recipient) => {
                if recipient.is_published() {
                    if let Some(player) = recipient.player() {
                        player.remove_bossbar(self.bossbar_uuid);
                    }
                    recipient.retire();
                }
            }
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
                if let Some(recipient) = state.recipients.iter().find(|r| r.matches(player)) {
                    if !recipient.is_published() {
                        state
                            .actions
                            .push(BossbarAction::Add(recipient.clone(), bar.clone()));
                    }
                } else {
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

// Finish committed successors before propagating a native callback panic.
struct WitherExecution<'a> {
    owner: &'a WitherEntity,
    world: &'a Arc<World>,
    actions: std::collections::VecDeque<BossbarAction>,
    completed: bool,
}

impl WitherExecution<'_> {
    fn run(mut self) {
        let mut first_panic = None;
        loop {
            while let Some(action) = self.actions.front() {
                // Retry a failed delivery once, including terminal REMOVE, without
                // letting a persistently panicking recipient strand its successors.
                for _ in 0..2 {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.owner.execute_bossbar_action(action, self.world);
                    }));
                    let Err(payload) = result else {
                        break;
                    };
                    if first_panic.is_none() {
                        first_panic = Some(payload);
                    } else if let Err(nested) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
                    {
                        std::mem::forget(nested);
                    }
                }
                self.actions.pop_front();
            }
            let mut state = self
                .owner
                .bossbar_players
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.actions.extend(std::mem::take(&mut state.actions));
            if self.actions.is_empty() {
                state.executing = false;
                self.completed = true;
                drop(state);
                if let Some(payload) = first_panic {
                    std::panic::resume_unwind(payload);
                }
                return;
            }
        }
    }
}

impl Drop for WitherExecution<'_> {
    fn drop(&mut self) {
        if !self.completed {
            restore_dispatch(self.owner, self.world, std::mem::take(&mut self.actions));
        }
    }
}

struct WitherDispatch {
    owner: Arc<WitherEntity>,
    world: Arc<World>,
    actions: Option<std::collections::VecDeque<BossbarAction>>,
}

impl WitherDispatch {
    fn run(mut self) {
        WitherExecution {
            owner: &self.owner,
            world: &self.world,
            actions: self.actions.take().unwrap_or_default(),
            completed: false,
        }
        .run();
    }
}

impl Drop for WitherDispatch {
    fn drop(&mut self) {
        if let Some(actions) = self.actions.take() {
            restore_dispatch(&self.owner, &self.world, actions);
        }
    }
}

fn restore_dispatch(
    owner: &WitherEntity,
    world: &Arc<World>,
    actions: std::collections::VecDeque<BossbarAction>,
) {
    let mut state = owner
        .bossbar_players
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut actions: Vec<_> = actions.into();
    actions.append(&mut state.actions);
    state.actions = actions;
    state.executing = false;
    drop(state);
    if let Some(owner) = owner.self_reference.upgrade() {
        let world = world.clone();
        // An abandoned GuardEffects still borrows its TLS queue while dropping us.
        // Retry independently, after unlocking, even when this owner will never tick again.
        std::thread::spawn(move || owner.with_bossbar(&world, |_| {}));
    }
}
