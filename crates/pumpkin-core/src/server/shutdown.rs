use super::Server;

impl Server {
    /// Starts the management shutdown transition before the server saves.
    ///
    /// Mirrors `DedicatedServer.stopServer` and
    /// `JsonRpcNotificationService.serverShuttingDown` ordering before saves.
    pub(crate) fn begin_shutdown(&self) {
        self.management_hub.broadcast_server_stopping();
    }

    #[cfg(test)]
    pub(crate) fn begin_shutdown_with_adapter(&self, after_claim: impl FnOnce()) {
        self.management_hub
            .broadcast_server_stopping_with_adapter(after_claim);
    }
}
