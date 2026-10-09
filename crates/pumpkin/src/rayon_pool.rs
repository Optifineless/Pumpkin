pub(crate) fn worker_pool_builder() -> rayon::ThreadPoolBuilder {
    rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("Rayon-Worker-{i}"))
        // Rayon Registry::catch_unwind aborts unhandled detached-task panics.
        // The server panic hook starts shutdown; returning here lets it finish.
        .panic_handler(|payload| {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<unknown>");
            tracing::error!("Panic on a Rayon worker: {message}");
        })
}

#[cfg(test)]
mod tests {
    use std::{process::Command, sync::mpsc, time::Duration};

    const CHILD: &str = "PUMPKIN_TEST_DETACHED_RAYON_PANIC";

    #[test]
    fn detached_rayon_panic_does_not_abort() {
        if std::env::var_os(CHILD).is_some() {
            super::worker_pool_builder()
                .num_threads(1)
                .build_global()
                .expect("configure the production worker pool");
            let (started, receiver) = mpsc::channel();
            rayon::spawn(move || {
                started.send(()).expect("notify test thread");
                std::panic::resume_unwind(Box::new("detached worker regression"));
            });
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("detached task must start");
            // With one worker this task can finish only after the panic is handled.
            let (finished, receiver) = mpsc::channel();
            rayon::spawn(move || finished.send(()).expect("notify test thread"));
            receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("the worker must survive its detached panic");
            return;
        }
        let result = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "rayon_pool::tests::detached_rayon_panic_does_not_abort",
                "--nocapture",
            ])
            .current_dir(std::env::temp_dir())
            .env(CHILD, "1")
            .output()
            .expect("run isolated panic regression");
        assert!(
            result.status.success(),
            "detached Rayon panic aborted subprocess: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
