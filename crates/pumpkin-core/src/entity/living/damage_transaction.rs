//! Serial combat segments, with an explicit release/reacquire boundary at plugin dispatch.
//! Only one entity can be owned by a thread. Entering another entity suspends the first;
//! callbacks suspend all local scopes, including native callbacks polled on this thread.
//! A resumed segment must read live state. Revisions invalidate calculations across callbacks.
//! Attack ledgers count only mutations made by that attack, excluding suspended callbacks.

#[cfg(test)]
pub mod test_hooks;

use pumpkin_util::math::vector3::Vector3;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed},
    },
};

#[derive(Default)]
pub(super) struct DamageOwner(Arc<OwnerState>);

#[cfg(test)]
mod review3_tests;

#[derive(Default)]
struct OwnerState {
    occupied: Mutex<bool>,
    waiting: AtomicUsize,
    #[cfg(test)]
    entries: AtomicUsize,
    #[cfg(test)]
    runtime_checks: AtomicUsize,
    #[cfg(test)]
    scope_allocations: AtomicUsize,
    available: Condvar,
    revision: AtomicU64,
    hurts: AtomicUsize,
    lifecycle: AtomicU64,
    committed_admission: crossbeam::atomic::AtomicCell<f32>,
}

struct Scope {
    owner: Arc<OwnerState>,
    lifecycle: u64,
    health_delta: Cell<f32>,
    // Independent gates preserve an outer hurtServer continuation when a nested call ends.
    minimum_health: RefCell<Vec<Option<f32>>>,
    depth: Cell<u32>,
    melee: Cell<bool>,
    motion: RefCell<Vec<PendingMotion>>,
}

pub(super) struct PendingHurt(Arc<OwnerState>);

impl Drop for PendingHurt {
    fn drop(&mut self) {
        if self.0.hurts.fetch_sub(1, Relaxed) == 1 {
            self.0.committed_admission.store(0.0);
        }
    }
}

pub enum PendingMotion {
    Knockback(f64, f64, f64),
    Impulse(Vector3<f64>),
    Mark,
}

thread_local! {
    static SCOPES: RefCell<Vec<Rc<Scope>>> = const { RefCell::new(Vec::new()) };
    static SPARE_SCOPE: RefCell<Option<Rc<Scope>>> = const { RefCell::new(None) };
}

#[must_use]
pub struct DamageToken {
    scope: Rc<Scope>,
    suspended: Option<SuspendedDamage>,
    nested: bool,
    depth: u32,
}

/// Releases combat ownership while external code runs; dropping this reacquires it.
/// Must surround the blocking bridge itself, before polling or dispatching guest work.
#[must_use]
pub struct SuspendedDamage(Vec<Rc<Scope>>);

impl OwnerState {
    fn acquire(&self) {
        let mut occupied = self
            .occupied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*occupied {
            *occupied = true;
            return;
        }
        self.waiting.fetch_add(1, Relaxed);
        drop(occupied);
        let wait = || {
            let mut occupied = self
                .occupied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while *occupied {
                occupied = self
                    .available
                    .wait(occupied)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            self.waiting.fetch_sub(1, Relaxed);
            *occupied = true;
        };
        #[cfg(test)]
        self.runtime_checks.fetch_add(1, Relaxed);
        if tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        }) {
            tokio::task::block_in_place(wait);
        } else {
            wait();
        }
    }

    fn release(&self) {
        *self
            .occupied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        if self.waiting.load(Relaxed) != 0 {
            self.available.notify_one();
        }
    }
}

pub fn suspend_damage() -> SuspendedDamage {
    let scopes = SCOPES.with(|current| std::mem::take(&mut *current.borrow_mut()));
    if let Some(scope) = scopes.first() {
        scope.owner.release();
    }
    SuspendedDamage(scopes)
}

impl Drop for SuspendedDamage {
    fn drop(&mut self) {
        if let Some(scope) = self.0.first() {
            scope.owner.acquire();
            SCOPES.with(|current| {
                debug_assert!(current.borrow().is_empty());
                *current.borrow_mut() = std::mem::take(&mut self.0);
            });
        }
    }
}

impl DamageOwner {
    pub(super) fn enter(&self) -> DamageToken {
        self.enter_scope(false)
    }

    fn enter_scope(&self, separate_ledger: bool) -> DamageToken {
        #[cfg(test)]
        self.0.entries.fetch_add(1, Relaxed);
        let (nested, different) = SCOPES.with(|current| {
            let current = current.borrow();
            (
                current
                    .last()
                    .is_some_and(|scope| Arc::ptr_eq(&scope.owner, &self.0)),
                !current.is_empty(),
            )
        });
        // Lock-only nesting reuses the active scope; melee attacks keep independent ledgers.
        if nested
            && !separate_ledger
            && let Some(scope) = SCOPES.with(|current| current.borrow().last().cloned())
            && scope.lifecycle == self.lifecycle()
        {
            let depth = scope.depth.get() + 1;
            scope.depth.set(depth);
            scope.minimum_health.borrow_mut().push(None);
            return DamageToken {
                scope,
                suspended: None,
                nested: true,
                depth,
            };
        }
        let suspended = (!nested && different).then(suspend_damage);
        if !nested {
            self.0.acquire();
        }
        let scope = self.reuse_scope();
        SCOPES.with(|current| current.borrow_mut().push(scope.clone()));
        DamageToken {
            scope,
            suspended,
            nested: false,
            depth: 0,
        }
    }

    fn reuse_scope(&self) -> Rc<Scope> {
        let spare = SPARE_SCOPE.with(|spare| spare.borrow_mut().take());
        if let Some(mut scope) = spare
            && let Some(reset) = Rc::get_mut(&mut scope)
        {
            reset.owner = self.0.clone();
            reset.lifecycle = self.lifecycle();
            reset.health_delta.set(0.0);
            reset.minimum_health.get_mut().clear();
            reset.minimum_health.get_mut().push(None);
            reset.depth.set(0);
            reset.melee.set(false);
            reset.motion.get_mut().clear();
            return scope;
        }
        #[cfg(test)]
        self.0.scope_allocations.fetch_add(1, Relaxed);
        Rc::new(Scope {
            owner: self.0.clone(),
            lifecycle: self.lifecycle(),
            health_delta: Cell::new(0.0),
            minimum_health: RefCell::new(vec![None]),
            depth: Cell::new(0),
            melee: Cell::new(false),
            motion: RefCell::default(),
        })
    }

    #[cfg(test)]
    pub(super) fn wait_until_contended(&self) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.0.waiting.load(Relaxed) == 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::yield_now();
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.0.entries.load(Relaxed)
    }

    pub(super) fn reset(&self) {
        self.0.lifecycle.fetch_add(1, Relaxed);
        self.0.committed_admission.store(0.0);
    }

    pub(super) fn lifecycle(&self) -> u64 {
        self.0.lifecycle.load(Relaxed)
    }

    pub(super) fn changed(&self, health_delta: f32) {
        self.0.revision.fetch_add(1, Relaxed);
        SCOPES.with(|current| {
            for scope in current.borrow().iter().filter(|scope| {
                Arc::ptr_eq(&scope.owner, &self.0) && scope.lifecycle == self.lifecycle()
            }) {
                scope
                    .health_delta
                    .set(scope.health_delta.get() + health_delta);
            }
        });
    }

    pub(super) fn pending_hurt(&self) -> PendingHurt {
        self.0.hurts.fetch_add(1, Relaxed);
        PendingHurt(self.0.clone())
    }

    pub(super) fn has_pending_hurt(&self) -> bool {
        self.0.hurts.load(Relaxed) != 0
    }

    pub(super) fn admits_health(&self, health: f32) -> bool {
        SCOPES.with(|scopes| {
            scopes.borrow().iter().all(|scope| {
                !Arc::ptr_eq(&scope.owner, &self.0)
                    || scope
                        .minimum_health
                        .borrow()
                        .iter()
                        .all(|minimum| minimum.is_none_or(|minimum| health > minimum))
            })
        })
    }

    pub(super) fn admission_baseline(&self, last_hurt: f32) -> f32 {
        last_hurt.max(self.0.committed_admission.load())
    }

    pub(super) fn reserve_admission(&self, amount: f32) {
        // Keep lastHurt's vanilla post-actuallyHurt assignment visible to callbacks, while
        // preventing another attacker from applying an already committed excess twice.
        self.0
            .committed_admission
            .store(self.0.committed_admission.load().max(amount));
    }

    pub(super) fn revision(&self) -> u64 {
        self.0.revision.load(Relaxed)
    }

    pub(super) fn defer_motion(&self, motion: PendingMotion) -> bool {
        SCOPES.with(|current| {
            let current = current.borrow();
            current
                .iter()
                .rev()
                .find(|scope| {
                    scope.melee.get()
                        && Arc::ptr_eq(&scope.owner, &self.0)
                        && scope.lifecycle == self.lifecycle()
                })
                .is_some_and(|scope| {
                    scope.motion.borrow_mut().push(motion);
                    true
                })
        })
    }
}

impl DamageToken {
    pub(crate) fn require_health_above(&self, minimum: f32) {
        let mut gates = self.scope.minimum_health.borrow_mut();
        let gate = &mut gates[self.depth as usize];
        *gate = Some(gate.map_or(minimum, |old| old.max(minimum)));
    }

    pub(crate) fn health_damage(&self) -> f32 {
        if self.is_current_life() {
            self.scope.health_delta.get()
        } else {
            0.0
        }
    }

    /// Checks a continuation's captured life while its target ownership is held.
    pub(crate) fn is_current_life(&self) -> bool {
        self.scope.lifecycle == self.scope.owner.lifecycle.load(Relaxed)
    }

    pub(crate) fn melee(&self) {
        self.scope.melee.set(true);
    }

    pub(crate) fn finish_motion(&self, living: &super::LivingEntity) -> Vector3<f64> {
        // Player.causeExtraKnockback: capture/apply/send/restore without a callback boundary.
        self.scope.melee.set(false);
        let old = living.entity.velocity.load();
        if self.scope.lifecycle != living.damage_owner.lifecycle() {
            self.scope.motion.borrow_mut().clear();
            return old;
        }
        for motion in self.scope.motion.take() {
            match motion {
                PendingMotion::Knockback(strength, x, z) => {
                    living.entity.apply_knockback(strength, x, z);
                }
                PendingMotion::Impulse(impulse) => living.entity.push_impulse(impulse),
                PendingMotion::Mark => living.entity.hurt_marked.store(true, Relaxed),
            }
        }
        old
    }
}

impl Drop for DamageToken {
    fn drop(&mut self) {
        if self.nested {
            debug_assert_eq!(self.scope.depth.get(), self.depth);
            self.scope.depth.set(self.scope.depth.get() - 1);
            self.scope.minimum_health.borrow_mut().pop();
            return;
        }
        debug_assert_eq!(self.scope.depth.get(), 0, "nested damage token still alive");
        SCOPES.with(|current| {
            let mut current = current.borrow_mut();
            let scope = current.pop();
            debug_assert!(
                scope
                    .as_ref()
                    .is_some_and(|scope| Rc::ptr_eq(scope, &self.scope))
            );
            if current.is_empty() {
                self.scope.owner.release();
            }
        });
        SPARE_SCOPE.with(|spare| *spare.borrow_mut() = Some(self.scope.clone()));
        // Reacquire an outer, different entity only after releasing this one.
        drop(self.suspended.take());
    }
}

impl super::LivingEntity {
    /// Identifies the life currently protected by combat ownership.
    pub(crate) fn damage_lifecycle(&self) -> u64 {
        self.damage_owner.lifecycle()
    }

    /// Checks the dying life after callbacks, clearing its death guard if that same life was healed.
    pub(crate) fn death_lifecycle_current(&self, lifecycle: u64) -> bool {
        if self.damage_owner.lifecycle() != lifecycle {
            return false;
        }
        if self.health.load() > 0.0 {
            // LivingEntity.die guards on dead, while hurtServer/tickDeath use health.
            // A callback that heals this same life must permit a later killing blow.
            self.dead.store(false, Relaxed);
            return false;
        }
        self.dead.load(Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn damage_entry_count(&self) -> usize {
        self.damage_owner.entry_count()
    }

    #[cfg(test)]
    pub(crate) fn wait_until_damage_contended(&self) -> bool {
        self.damage_owner.wait_until_contended()
    }

    /// Runs a synchronous read/modify/write under this entity's combat ownership.
    /// Plugin dispatch releases ownership; read live values again after invoking callbacks.
    pub fn with_damage_owned<R>(&self, action: impl FnOnce() -> R) -> R {
        let _owner = self.damage_owner.enter();
        action()
    }

    pub(crate) fn own_damage(&self) -> DamageToken {
        self.damage_owner.enter()
    }

    pub(crate) fn begin_melee(&self) -> DamageToken {
        let owner = self.damage_owner.enter_scope(true);
        owner.melee();
        owner
    }

    pub(crate) fn defer_hurt_knockback(&self, strength: f64, x: f64, z: f64) -> bool {
        self.damage_owner
            .defer_motion(PendingMotion::Knockback(strength, x, z))
    }

    pub(crate) fn mark_hurt(&self) {
        let _owner = self.own_damage();
        if !self.damage_owner.defer_motion(PendingMotion::Mark) {
            self.entity.hurt_marked.store(true, Relaxed);
        }
    }

    pub(crate) fn push_hurt(&self, impulse: Vector3<f64>) {
        let _owner = self.own_damage();
        if !self
            .damage_owner
            .defer_motion(PendingMotion::Impulse(impulse))
        {
            self.entity.push_impulse(impulse);
        }
    }

    // LivingEntity.heal: read and addition are inside the owner, after the regain event.
    pub(super) fn heal_in_transaction(&self, amount: f32) {
        let _owner = self.damage_owner.enter();
        let health = self.health.load();
        if health > 0.0 {
            self.set_health(health + amount);
        }
    }

    // LivingEntity.tickDeath cannot advance while a lethal transaction is awaiting protection.
    pub(super) fn tick_damage_timers(&self) -> DamageToken {
        let owner = self.damage_owner.enter();
        if self.hurt_cooldown.load(Relaxed) > 0 {
            self.hurt_cooldown.fetch_sub(1, Relaxed);
        }
        if self.hurt_time.load(Relaxed) > 0 {
            self.hurt_time.fetch_sub(1, Relaxed);
        }
        owner
    }
}
