//! Synchronous effect batches and ordered notifications for state owners.
//!
//! Start a batch BEFORE acquiring its owner guard; it must end AFTER all guards
//! are released, nested batches must drop in reverse creation order, and no batch
//! may cross await. Only reviewed effect producers opt in;
//! plugin dispatch itself is never deferred. Nested mutations append to the outer
//! batch so a helper cannot flush while its caller still holds an owner guard.

use std::{cell::RefCell, marker::PhantomData, rc::Rc, sync::Arc};

use bytes::Bytes;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{BClientPacket, ClientPacket};
use pumpkin_util::sync::assert_no_guards_held;

use crate::{net::ClientPlatform, world::World};

type Effect = Box<dyn FnOnce() + Send>;
thread_local! {
    static BATCHES: RefCell<Vec<Vec<Effect>>> = const { RefCell::new(Vec::new()) };
}

/// Collects reviewed effects until the caller releases its synchronous owner guards.
pub struct GuardEffects(PhantomData<Rc<()>>);

impl GuardEffects {
    #[must_use]
    pub fn new() -> Self {
        BATCHES.with_borrow_mut(|batches| batches.push(Vec::new()));
        Self(PhantomData)
    }

    fn take(self) -> Vec<Effect> {
        let effects = BATCHES.with_borrow_mut(|batches| batches.pop().unwrap_or_default());
        std::mem::forget(self);
        effects
    }
}

/// A state owner's notification queue, enqueued while that owner's guard is held.
#[derive(Debug, Default)]
pub(crate) struct OwnerEffects(Arc<OrderedEffects>);

impl Clone for OwnerEffects {
    fn clone(&self) -> Self {
        // A cloned scoreboard/border is independent state, not another notification owner.
        Self::default()
    }
}

impl OwnerEffects {
    /// Captures notifications under the owner guard in mutation order.
    /// Start a `GuardEffects` batch before taking that guard; never cross await.
    pub(crate) fn capture(&self, operation: impl FnOnce()) {
        self.0.capture(operation);
    }
}

#[derive(Default)]
struct OrderedEffects(std::sync::Mutex<Delivery>);

#[derive(Default)]
struct Delivery {
    effects: std::collections::VecDeque<PendingEffects>,
    draining: bool,
}

struct PendingEffects {
    ready: Arc<std::sync::atomic::AtomicBool>,
    effects: Vec<Effect>,
}

impl std::fmt::Debug for OrderedEffects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrderedEffects").finish_non_exhaustive()
    }
}

impl OrderedEffects {
    /// Captures reviewed sends in mutation order and schedules delivery after unlocking.
    /// The caller must start a `GuardEffects` batch before acquiring the owner guard.
    pub(crate) fn capture(self: &Arc<Self>, operation: impl FnOnce()) {
        let batch = GuardEffects::new();
        operation();
        let effects = batch.take();
        if effects.is_empty() {
            return;
        }
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .effects
            .push_back(PendingEffects {
                ready: ready.clone(),
                effects,
            });
        let request = DispatchRequest {
            owner: self.clone(),
            ready,
        };
        let effect = move || request.run();
        if let Err(effect) = defer(effect) {
            // OwnerMutex batches attached owners; standalone delivery stays asynchronous.
            std::thread::spawn(effect);
        }
    }

    fn drain(self: &Arc<Self>) {
        assert_no_guards_held("OrderedEffects::dispatch");
        {
            let mut delivery = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if delivery.draining {
                return;
            }
            delivery.draining = true;
        };
        let _reset = DrainReset(self);
        loop {
            let effect = {
                let mut delivery = self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let effect =
                    if delivery.effects.front().is_some_and(|pending| {
                        pending.ready.load(std::sync::atomic::Ordering::Acquire)
                    }) {
                        delivery.effects.pop_front()
                    } else {
                        None
                    };
                if effect.is_none() {
                    delivery.draining = false;
                    return;
                }
                effect
            };
            if let Some(pending) = effect {
                for effect in pending.effects {
                    effect();
                }
            }
        }
    }
}

struct DispatchRequest {
    owner: Arc<OrderedEffects>,
    ready: Arc<std::sync::atomic::AtomicBool>,
}

impl DispatchRequest {
    fn run(self) {
        // Only the outer batch can mark this mutation ready, after its guards unlock.
        self.ready.store(true, std::sync::atomic::Ordering::Release);
        self.owner.drain();
    }
}

impl Drop for DispatchRequest {
    fn drop(&mut self) {
        if self.ready.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        // An unwinding batch discards its effects; do not retain an unready queue head.
        let (pending, needs_drain) = {
            let mut delivery = self
                .owner
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = delivery
                .effects
                .iter()
                .position(|pending| Arc::ptr_eq(&pending.ready, &self.ready));
            let pending = index.and_then(|index| delivery.effects.remove(index));
            let needs_drain = !delivery.draining
                && delivery.effects.front().is_some_and(|pending| {
                    pending.ready.load(std::sync::atomic::Ordering::Acquire)
                });
            (pending, needs_drain)
        };
        drop(pending);
        if needs_drain {
            // WorldBorder.setSize, ServerScoreboard.onScoreChanged and PlayerAdvancements.flushDirty
            // deliver every change; abandoning a batch must not strand later notifications.
            // Ready successors have already unlocked. Dispatch away from the unwinding
            // thread, where nested batches may still be releasing their guards.
            let owner = self.owner.clone();
            std::thread::spawn(move || owner.drain());
        }
    }
}

struct DrainReset<'a>(&'a Arc<OrderedEffects>);

impl Drop for DrainReset<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let needs_drain = {
            let mut delivery = self
                .0
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            delivery.draining = false;
            delivery
                .effects
                .front()
                .is_some_and(|pending| pending.ready.load(std::sync::atomic::Ordering::Acquire))
        };
        if needs_drain {
            // The failed effect was already removed. Recover ready successors without
            // replaying it or invoking native callbacks on the unwinding thread.
            let owner = self.0.clone();
            std::thread::spawn(move || owner.drain());
        }
    }
}

impl Drop for GuardEffects {
    fn drop(&mut self) {
        let effects = BATCHES.with_borrow_mut(|batches| {
            let effects = batches.pop().unwrap_or_default();
            if std::thread::panicking() {
                return Vec::new();
            }
            if let Some(parent) = batches.last_mut() {
                parent.extend(effects);
                Vec::new()
            } else {
                effects
            }
        });
        if !effects.is_empty() {
            assert_no_guards_held("GuardEffects::dispatch");
            for effect in effects {
                effect();
            }
        }
    }
}

/// Returns the effect unchanged when no explicit batch owns this operation.
pub(crate) fn defer<F: FnOnce() + Send + 'static>(effect: F) -> Result<(), F> {
    BATCHES.with_borrow_mut(|batches| {
        if let Some(batch) = batches.last_mut() {
            batch.push(Box::new(effect));
            Ok(())
        } else {
            Err(effect)
        }
    })
}

pub(crate) fn is_active() -> bool {
    BATCHES.with_borrow(|batches| !batches.is_empty())
}

pub(crate) fn send(client: Arc<ClientPlatform>, bytes: Bytes) {
    let effect = move || match client.as_ref() {
        ClientPlatform::Java(client) => client.try_enqueue_packet(bytes),
        ClientPlatform::Bedrock(client) => client.try_enqueue_packet(bytes),
    };
    if let Err(effect) = defer(effect) {
        effect();
    }
}

pub(crate) fn broadcast_je(world: &World, packet: &impl ClientPacket) {
    if let Some(bytes) = serialize_je(packet) {
        for player in world.players.load().iter() {
            if player.client.java().is_some() {
                send(player.client.clone(), bytes.clone());
            }
        }
    }
}

fn serialize_je<P: ClientPacket>(packet: &P) -> Option<Bytes> {
    match packet.serialize_packet(&CURRENT_MC_VERSION) {
        Ok(bytes) => Some(bytes),
        Err(pumpkin_protocol::ser::WritingError::UnsupportedVersion(_)) => None,
        Err(error) => {
            tracing::error!(
                "Failed to serialize packet {} for version {:?}: {}",
                std::any::type_name::<P>(),
                CURRENT_MC_VERSION,
                error
            );
            None
        }
    }
}

pub(crate) fn send_editioned(
    client: Arc<ClientPlatform>,
    je: &impl ClientPacket,
    be: &impl BClientPacket,
) {
    let bytes = match client.as_ref() {
        ClientPlatform::Java(client) => client.serialize_packet(je).ok(),
        ClientPlatform::Bedrock(client) => client.serialize_packet(be).ok(),
    };
    if let Some(bytes) = bytes {
        send(client, bytes);
    }
}

pub(crate) fn broadcast_editioned(world: &World, je: &impl ClientPacket, be: &impl BClientPacket) {
    let java = serialize_je(je);
    for player in world.players.load().iter() {
        let bytes = match player.client.as_ref() {
            ClientPlatform::Java(_) => java.clone(),
            ClientPlatform::Bedrock(client) => match client.serialize_packet(be) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    tracing::error!("Failed to serialize bedrock packet: {}", error);
                    None
                }
            },
        };
        if let Some(bytes) = bytes {
            send(player.client.clone(), bytes);
        }
    }
}

impl Default for GuardEffects {
    fn default() -> Self {
        Self::new()
    }
}
