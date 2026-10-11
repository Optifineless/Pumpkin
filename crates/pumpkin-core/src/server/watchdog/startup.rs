use crate::PumpkinServer;

use super::WatchdogGuard;
use crate::server::ticker::Ticker;

impl PumpkinServer {
    // DedicatedServer.initServer / MinecraftServer.runServer: detection is ready
    // before the first gameplay tick, after world generation and plugin setup.
    pub(crate) fn start_ticker(&self) -> std::io::Result<Option<WatchdogGuard>> {
        let mut ticker = self
            .ticker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ticker.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Ticker is already running",
            ));
        }
        let watchdog = self.server.start_watchdog()?;
        let server = self.server.clone();
        let (ready, wait_ready) = std::sync::mpsc::sync_channel(0);
        *ticker = Some(
            std::thread::Builder::new()
                .name("Server-Ticker".into())
                .spawn(move || {
                    if wait_ready.recv().is_ok() {
                        Ticker::run(&server);
                    }
                })?,
        );
        // Publish the thread handle and release its lock before any plugin callback.
        drop(ticker);
        let _ = ready.send(());
        Ok(watchdog)
    }
}
