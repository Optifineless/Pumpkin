use super::Server;

impl Server {
    /// Notifies management clients before cancellation drains their sessions.
    ///
    /// Mirrors `DedicatedServer.stopServer` and
    /// `JsonRpcNotificationService.serverShuttingDown` ordering before saves.
    pub(crate) fn begin_shutdown(&self) {
        self.begin_shutdown_with(|| self.management_hub.try_broadcast_server_stopping());
    }

    fn begin_shutdown_with(&self, broadcast_stopping: impl FnOnce() -> bool) {
        if !broadcast_stopping() {
            return;
        }
        self.management_shutdown.cancel();
    }

    #[cfg(test)]
    pub(crate) fn begin_shutdown_with_adapter(&self, after_claim: impl FnOnce()) {
        self.begin_shutdown_with(|| {
            self.management_hub
                .broadcast_server_stopping_with_adapter(after_claim)
        });
    }
}
