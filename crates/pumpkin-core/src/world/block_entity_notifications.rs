//! Client tags follow the same commit order as persistent block-entity images.

use pumpkin_protocol::codec::item_nbt::project_item_nbt_for_client;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{VarInt, java::client::play::CBlockEntityData};
use pumpkin_util::math::position::BlockPos;

use super::World;
use crate::block::entities::BlockEntity;

pub(super) struct BlockEntityNotification {
    ready: Arc<AtomicBool>,
    position: BlockPos,
    expected: Weak<dyn BlockEntity>,
    bytes: bytes::Bytes,
}

pub(super) struct NotificationRequest {
    world: Weak<World>,
    ready: Arc<AtomicBool>,
}

impl World {
    fn notification_world(&self) -> Option<Arc<Self>> {
        self.server
            .upgrade()?
            .worlds
            .load()
            .iter()
            .find(|world| std::ptr::eq(world.as_ref(), self))
            .cloned()
    }

    pub(super) fn prepare_block_entity_notification(
        &self,
        position: BlockPos,
        entity: &Arc<dyn BlockEntity>,
        nbt: Option<NbtCompound>,
    ) -> Option<(BlockEntityNotification, NotificationRequest)> {
        let nbt = nbt?;
        let world = self.notification_world()?;
        let ready = Arc::new(AtomicBool::new(false));
        let notification = BlockEntityNotification {
            ready: ready.clone(),
            position,
            expected: Arc::downgrade(entity),
            bytes: pumpkin_nbt::Nbt::from(project_item_nbt_for_client(nbt)).write_unnamed(),
        };
        Some((
            notification,
            NotificationRequest {
                world: Arc::downgrade(&world),
                ready,
            },
        ))
    }

    fn drain_block_entity_notifications(&self) {
        let mut loads = self
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if loads.notifying {
            return;
        }
        loads.notifying = true;
        drop(loads);
        let _reset = NotificationDrain(self);
        loop {
            let notification = {
                let mut loads = self
                    .pending_block_entity_loads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if loads
                    .notifications
                    .front()
                    .is_some_and(|pending| pending.ready.load(Ordering::Acquire))
                {
                    loads.notifications.pop_front()
                } else {
                    loads.notifying = false;
                    None
                }
            };
            let Some(notification) = notification else {
                return;
            };
            // No native accessor, callback or send runs under admission ownership.
            let current = self
                .block_entities
                .get(&notification.position.chunk_position())
                .and_then(|entities| entities.get(&notification.position).cloned());
            if let Some(current) = current
                .filter(|entity| Weak::ptr_eq(&Arc::downgrade(entity), &notification.expected))
            {
                self.broadcast_to_chunk(
                    notification.position.chunk_position(),
                    &CBlockEntityData::new(
                        notification.position,
                        VarInt(current.get_id() as i32),
                        notification.bytes.as_ref().into(),
                    ),
                );
            }
        }
    }
}

impl NotificationRequest {
    pub(super) fn dispatch(self) {
        let deliver = move || {
            self.ready.store(true, Ordering::Release);
            if let Some(world) = self.world.upgrade() {
                world.drain_block_entity_notifications();
            }
        };
        if let Err(deliver) = super::guard_effects::defer(deliver) {
            deliver();
        }
    }
}

impl Drop for NotificationRequest {
    fn drop(&mut self) {
        if self.ready.load(Ordering::Acquire) {
            return;
        }
        let Some(world) = self.world.upgrade() else {
            return;
        };
        let needs_drain = {
            let loads = world
                .pending_block_entity_loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !loads
                .notifications
                .iter()
                .any(|pending| Arc::ptr_eq(&pending.ready, &self.ready))
            {
                return;
            }
            // Publication already committed. Abandoning its outer batch must not
            // discard the final client image or strand ready successors.
            self.ready.store(true, Ordering::Release);
            !loads.notifying
                && loads
                    .notifications
                    .front()
                    .is_some_and(|pending| pending.ready.load(Ordering::Acquire))
        };
        // GuardEffects may still borrow its TLS queue on this unwinding thread.
        if needs_drain {
            std::thread::spawn(move || world.drain_block_entity_notifications());
        }
    }
}

struct NotificationDrain<'a>(&'a World);

impl Drop for NotificationDrain<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        self.0
            .pending_block_entity_loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .notifying = false;
        if let Some(world) = self.0.notification_world() {
            std::thread::spawn(move || world.drain_block_entity_notifications());
        }
    }
}
