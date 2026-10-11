//! Explicit synchronous effect batches for menu and scoreboard mutations.
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
