use super::Entity;
use crate::{net::java::JavaClient, world::World};
use pumpkin_data::entity::EntityType;
use pumpkin_protocol::java::client::play::CSetEntityMetadata;

impl Entity {
    pub(super) fn flush_dirty_entity_data(&self) {
        // ServerEntity.sendDirtyEntityData sends the same packDirty snapshot to all viewers.
        let Some(batch) = self.synched_data.pack_dirty() else {
            return;
        };
        // Vanilla consumes packDirty before packet callbacks can re-enter tracking.
        self.synched_data.clear_dirty(&batch);

        let world = self.world.load();
        let players = world.players.load();

        // Unwatched, skip the player scan.
        if self.entity_type != &EntityType::PLAYER
            && world
                .entity_tracker
                .get_tracked_entity(self.entity_id)
                .is_some_and(|tracked| tracked.seen_by.is_empty())
        {
            return;
        }

        let java_recipients = self.java_metadata_recipients(&world, &players);

        if java_recipients.is_empty() {
            // Vanilla `packDirty` clears even without recipients.
            return;
        }

        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());

        for (version, recipients) in recipients_by_version {
            if let Some(buf) = batch.pack_for_version(&version) {
                let packet = CSetEntityMetadata::new(self.entity_id.into(), buf);
                if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version)
                {
                    for recipient in recipients {
                        recipient.try_enqueue_packet(packet_data.clone());
                    }
                }
            }
        }
    }
    pub(super) fn resend_synced_entity_data(&self) {
        let batch = self.synched_data.get_non_default_values();
        let world = self.world.load();
        let players = world.players.load();

        let java_recipients = self.java_metadata_recipients(&world, &players);

        if java_recipients.is_empty() {
            return;
        }

        self.synched_data.clear_dirty(&batch);
        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());

        for (version, recipients) in recipients_by_version {
            if let Some(buf) = batch.pack_for_version(&version) {
                let packet = CSetEntityMetadata::new(self.entity_id.into(), buf);
                if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version)
                {
                    for recipient in recipients {
                        recipient.try_enqueue_packet(packet_data.clone());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "metadata_flush_tests.rs"]
mod tests;
