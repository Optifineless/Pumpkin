use super::{pending::PendingConnection, session::KeepAliveAction};
use crate::server::Server;
use pumpkin_protocol::{
    java::{
        client::config::{CConfigAddResourcePack, CConfigKeepAlive, CFinishConfig},
        server::config::{ResourcePackResponseResult, SConfigResourcePack, SKnownPacks},
    },
    ser::ReadingError,
};
use pumpkin_util::text::TextComponent;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum ConfigTask {
    #[default]
    NotStarted,
    SynchronizeRegistries,
    ResourcePack(uuid::Uuid),
    JoinWorld,
}

impl ConfigTask {
    // ServerConfigurationPacketListenerImpl.finishCurrentTask.
    pub(super) fn complete(&mut self, expected: Self) -> Result<(), ReadingError> {
        if *self != expected {
            return Err(ReadingError::Message(
                "Unexpected configuration task completion".into(),
            ));
        }
        *self = Self::NotStarted;
        Ok(())
    }
}

impl PendingConnection {
    pub(super) async fn keep_config_connection_alive(&mut self) -> bool {
        match self.keep_alive.poll(std::time::Instant::now()) {
            KeepAliveAction::Wait => true,
            KeepAliveAction::Send(keep_alive_id) => {
                self.send_packet_now(&CConfigKeepAlive { keep_alive_id })
                    .await
            }
            KeepAliveAction::Timeout => {
                self.kick(TextComponent::translate("disconnect.timeout", []))
                    .await;
                false
            }
        }
    }

    pub(super) async fn handle_config_keep_alive(&mut self, id: i64) {
        if self
            .keep_alive
            .acknowledge(id, std::time::Instant::now())
            .is_none()
        {
            self.kick(TextComponent::translate("disconnect.timeout", []))
                .await;
        }
    }

    pub(super) async fn handle_known_packs_response(
        &mut self,
        server: &Server,
        _packet: &SKnownPacks<'_>,
    ) -> Result<(), ReadingError> {
        // ServerConfigurationPacketListenerImpl.handleSelectKnownPacks completes only this task.
        self.config_task
            .complete(ConfigTask::SynchronizeRegistries)?;
        // SynchronizeRegistriesTask.handleResponse sends full registries on a mismatch.
        // Pumpkin always sends full entries, so no accepted-pack data is omitted.
        self.handle_known_packs(server).await;
        let resource = &server.advanced_config.resource_pack.java;
        if resource.enabled {
            let id = uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_DNS, resource.url.as_bytes());
            self.config_task = ConfigTask::ResourcePack(id);
            self.send_packet_now(&CConfigAddResourcePack::new(
                &id,
                &resource.url,
                &resource.sha1,
                resource.force,
                (!resource.prompt_message.is_empty())
                    .then(|| TextComponent::text(resource.prompt_message.clone())),
            ))
            .await;
        } else {
            self.finish_configuration().await;
        }
        Ok(())
    }

    pub async fn handle_resource_pack_response(
        &mut self,
        server: &Server,
        packet: SConfigResourcePack,
    ) {
        // ServerConfigurationPacketListenerImpl.handleResourcePackResponse: only terminal
        // responses finish the resource-pack task; DOWNLOADED still precedes loading.
        if self.config_task != ConfigTask::ResourcePack(packet.uuid) {
            self.kick(TextComponent::text("Unexpected resource pack response"))
                .await;
            return;
        }
        match packet.response_result() {
            ResourcePackResponseResult::Accepted | ResourcePackResponseResult::Downloaded => {}
            ResourcePackResponseResult::Unknown(_) => {
                self.kick(TextComponent::text("Invalid resource pack response"))
                    .await;
            }
            ResourcePackResponseResult::Declined
                if server.advanced_config.resource_pack.java.force =>
            {
                self.kick(TextComponent::translate(
                    "multiplayer.requiredTexturePrompt.disconnect",
                    [],
                ))
                .await;
            }
            _ => self.finish_configuration().await,
        }
    }

    async fn finish_configuration(&mut self) {
        if self.send_packet_now(&CFinishConfig).await {
            self.config_task = ConfigTask::JoinWorld;
            self.keep_alive.close_listener(std::time::Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_requires_the_expected_task_once() {
        let mut task = ConfigTask::SynchronizeRegistries;
        assert!(task.complete(ConfigTask::JoinWorld).is_err());
        assert!(task.complete(ConfigTask::SynchronizeRegistries).is_ok());
        assert!(task.complete(ConfigTask::SynchronizeRegistries).is_err());
        task = ConfigTask::ResourcePack(uuid::Uuid::nil());
        assert!(task.complete(ConfigTask::JoinWorld).is_err());
        task = ConfigTask::JoinWorld;
        assert!(task.complete(ConfigTask::JoinWorld).is_ok());
        assert!(task.complete(ConfigTask::JoinWorld).is_err());
    }
}
