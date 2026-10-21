use std::sync::Arc;

use pumpkin_data::BlockStateId;
use pumpkin_util::sync::{SnapshotBoundary as SB, assert_not_in_snapshot};

use super::{World, pending_block_entity::ObservedBlockEntityInstallation};
use crate::block::entities::{BlockEntity, jukebox::JukeboxBlockEntity};

impl World {
    /// Publishes delayed structure/feature NBT only for its observed terrain.
    pub(crate) fn add_block_entity_from_observed(
        &self,
        entity: Arc<dyn BlockEntity>,
        expected: BlockStateId,
    ) -> bool {
        assert_not_in_snapshot(SB::Mutation, "World::add_block_entity_from_observed");
        let position = entity.get_position();
        self.bind_block_entity_context(entity.as_ref());
        let stamp = self.next_block_entity_capture_stamp();
        let packet_nbt = entity.chunk_data_nbt();
        let persistent_nbt = self.capture_block_entity_snapshot(position, &entity);
        let (notification, request) = self
            .prepare_block_entity_notification(position, &entity, packet_nbt)
            .unzip();

        #[cfg(test)]
        self.before_state_write_test(position);
        let Some(mutation) = self.reserve_block_entity_mutation(position) else {
            return false;
        };
        let previous = self.install_block_entity_from_observed(position, expected, &entity);
        drop(mutation);
        let ObservedBlockEntityInstallation::Installed(previous) = previous else {
            return false;
        };
        if let Some(previous) = previous {
            Self::retire_replaced_block_entity(&previous, &entity);
        }
        #[cfg(test)]
        {
            let hook = self
                .persistence_test_hooks
                .registration
                .lock()
                .unwrap()
                .clone();
            if let Some(hook) = hook {
                hook();
            }
        }

        let Some(mutation) = self.reserve_block_entity_mutation(position) else {
            return true;
        };
        let published = self.publish_block_entity_nbt(
            position,
            &persistent_nbt,
            stamp,
            Some(&entity),
            notification,
        );
        drop(mutation);
        if published {
            if let Some(jukebox) = entity.as_any().downcast_ref::<JukeboxBlockEntity>() {
                jukebox.notify_playback_power_if_changed();
            }
            #[cfg(test)]
            {
                let hook = self
                    .persistence_test_hooks
                    .registration_published
                    .lock()
                    .unwrap()
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            if let Some(request) = request {
                request.dispatch();
            }
        }
        drop(entity);
        true
    }
}
