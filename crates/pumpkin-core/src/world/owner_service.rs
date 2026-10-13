//! Logical World service authority. This does not seal public Level mutation APIs.

use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use super::World;
use crate::server::Server;

const UNINITIALIZED: u8 = 0;
const CLOCK_READY: u8 = 1;
const PUBLIC_INGRESS: u8 = 2;
const IDLE: u64 = 0;
const CLAIMING: u64 = u64::MAX;

pub(super) struct OwnerServiceState {
    startup: AtomicU8,
    restored_age: i64,
    active: AtomicU64,
    epoch: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceEntryError {
    NotInitialized,
    ServiceAlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceMode {
    Normal,
    Frozen,
}

// Private fields and Rc's auto-traits keep authority on the enclosing service stack.
struct ServiceReceipt<'a> {
    state: &'a OwnerServiceState,
    epoch: u64,
    mode: ServiceMode,
    _local: PhantomData<Rc<()>>,
}

impl OwnerServiceState {
    pub(super) const fn new() -> Self {
        Self {
            startup: AtomicU8::new(UNINITIALIZED),
            restored_age: 0,
            active: AtomicU64::new(IDLE),
            epoch: AtomicU64::new(0),
        }
    }

    fn clock_ready(&mut self, restored_age: i64) -> bool {
        if *self.startup.get_mut() != UNINITIALIZED {
            return false;
        }
        self.restored_age = restored_age;
        self.startup.store(CLOCK_READY, Ordering::Release);
        true
    }

    fn public_ingress(&mut self, observed_age: i64) -> bool {
        if *self.startup.get_mut() != CLOCK_READY || self.restored_age != observed_age {
            return false;
        }
        self.startup.store(PUBLIC_INGRESS, Ordering::Release);
        true
    }

    fn enter(&self, mode: ServiceMode) -> Result<ServiceReceipt<'_>, ServiceEntryError> {
        if self.startup.load(Ordering::Acquire) != PUBLIC_INGRESS {
            return Err(ServiceEntryError::NotInitialized);
        }
        if self
            .active
            .compare_exchange(IDLE, CLAIMING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ServiceEntryError::ServiceAlreadyActive);
        }
        // Epoch issuance happens only after the exclusive claim; rejected entries cannot advance it.
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        assert!(epoch < CLAIMING, "World service epoch exhausted");
        self.active.store(epoch, Ordering::Release);
        Ok(ServiceReceipt {
            state: self,
            epoch,
            mode,
            _local: PhantomData,
        })
    }

    fn release(&self, epoch: u64) {
        let released =
            self.active
                .compare_exchange(epoch, IDLE, Ordering::Release, Ordering::Relaxed);
        debug_assert!(released.is_ok(), "World service receipt lost its claim");
    }
}

impl Drop for ServiceReceipt<'_> {
    fn drop(&mut self) {
        self.state.release(self.epoch);
    }
}

impl World {
    // ServerLevel construction restores levelData before MinecraftServer.tickChildren dispatches it.
    pub(super) fn initialize_owner_service(&mut self, restored_age: i64) {
        if !self.owner_service.clock_ready(restored_age) {
            tracing::error!(world = %self.uuid, "World service initialization repeated");
            return;
        }
        // LevelAccessor.getGameTime and scheduling must see the same restored ServerLevel clock.
        let observed_age = self.level.get_game_time();
        if !self.owner_service.public_ingress(observed_age) {
            tracing::error!(
                world = %self.uuid,
                restored_age = self.owner_service.restored_age,
                level_game_time = observed_age,
                "World service restored clock mismatch"
            );
        }
    }

    fn with_service<T>(
        &self,
        mode: ServiceMode,
        body: impl FnOnce() -> T,
    ) -> Result<T, ServiceEntryError> {
        let receipt = self.owner_service.enter(mode)?;
        debug_assert_eq!(receipt.mode, mode);
        pumpkin_util::sync::assert_no_guards_held("World service body");
        let result = body();
        drop(receipt);
        Ok(result)
    }

    /// Runs a synchronous normal service body after readiness and exclusive entry validation.
    pub(crate) fn with_normal_service<T>(
        &self,
        body: impl FnOnce() -> T,
    ) -> Result<T, ServiceEntryError> {
        self.with_service(ServiceMode::Normal, body)
    }

    /// Runs a synchronous frozen service body under the same claim as normal World ticking.
    pub(crate) fn with_frozen_service<T>(
        &self,
        body: impl FnOnce() -> T,
    ) -> Result<T, ServiceEntryError> {
        self.with_service(ServiceMode::Frozen, body)
    }

    pub(crate) fn report_service_violation(&self, error: ServiceEntryError) {
        tracing::error!(world = %self.uuid, ?error, "World service entry invariant violated");
        debug_assert!(false, "World service entry invariant violated: {error:?}");
    }

    // MinecraftServer.tickChildren serializes ServerLevel.tick; Pumpkin retains per-World parallelism.
    pub fn tick(self: &Arc<Self>, server: &Arc<Server>) {
        if let Err(error) = self.with_normal_service(|| self.tick_body(server)) {
            self.report_service_violation(error);
        }
    }
}

#[cfg(test)]
#[path = "owner_service_tests.rs"]
mod tests;
