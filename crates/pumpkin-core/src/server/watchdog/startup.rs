use crate::PumpkinServer;

use crate::server::ticker::Ticker;

impl PumpkinServer {
    // MinecraftServer.halt / stopServer finish tick mutations before final storage drains.
    pub(crate) async fn stop_ticker(&self) {
        crate::STOP_INTERRUPT.cancel();
        let ticker = self
            .ticker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(ticker) = ticker {
            let joined = tokio::task::spawn_blocking(move || ticker.join()).await;
            if !matches!(joined, Ok(Ok(()))) {
                tracing::error!("Server ticker failed during shutdown");
                crate::SERVER_EXIT_CODE.store(1, std::sync::atomic::Ordering::Release);
            }
            if joined.is_err() {
                // Without a completed OS-thread join, saving would race tick mutations.
                super::hard_exit();
            }
        }
        // The ticker has exited; final saves are intentionally unarmed.
        self.server.watchdog.stop();
    }

    // DedicatedServer.initServer / MinecraftServer.runServer: detection is ready
    // before the first gameplay tick. Pumpkin plugins may await ticks in on_load.
    pub(crate) fn start_ticker(&self) -> std::io::Result<()> {
        let mut ticker = self
            .ticker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ticker.is_some() {
            return Ok(());
        }
        let watchdog = self.server.start_watchdog()?;
        let server = self.server.clone();
        let (ready, wait_ready) = std::sync::mpsc::sync_channel(0);
        *ticker = Some(
            std::thread::Builder::new()
                .name("Server-Ticker".into())
                .spawn(move || {
                    // Keep detection alive throughout plugin loading and tick work.
                    let _watchdog = watchdog;
                    if wait_ready.recv().is_ok() {
                        Ticker::run(&server);
                    }
                })?,
        );
        // Publish the thread handle and release its lock before any plugin callback.
        drop(ticker);
        let _ = ready.send(());
        Ok(())
    }
}
