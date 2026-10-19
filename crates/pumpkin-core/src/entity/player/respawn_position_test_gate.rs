#![expect(clippy::unwrap_used, reason = "Bounded portal lookup regression gate")]

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use tokio::sync::oneshot;
use uuid::Uuid;

struct Gate {
    entered: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

static GATES: LazyLock<Mutex<HashMap<Uuid, Gate>>> = LazyLock::new(Mutex::default);

pub fn install(player: Uuid) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (entered, suspended) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    GATES.lock().unwrap().insert(
        player,
        Gate {
            entered,
            resume: resumed,
        },
    );
    (suspended, resume)
}

pub(super) async fn pause(player: Uuid) {
    let gate = { GATES.lock().unwrap().remove(&player) };
    if let Some(gate) = gate {
        let _ = gate.entered.send(());
        tokio::time::timeout(Duration::from_secs(45), gate.resume)
            .await
            .unwrap()
            .unwrap();
    }
}
