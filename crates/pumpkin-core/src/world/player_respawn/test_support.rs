//! Exact-player gates for the real respawn packet task; no registry guard crosses a wait.
use crate::entity::player::Player;
use std::{collections::HashMap, sync::Arc, sync::Mutex, sync::OnceLock, time::Duration};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, PartialEq, Eq)]
pub enum Phase {
    BeforeAdmission,
    AdmissionRejected,
}

pub struct Step {
    pub phase: Phase,
    pub release: oneshot::Sender<()>,
}

type Gates = HashMap<i32, mpsc::UnboundedSender<Step>>;
static GATES: OnceLock<Mutex<Gates>> = OnceLock::new();

pub struct Registration(i32);

impl Drop for Registration {
    fn drop(&mut self) {
        GATES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

pub fn capture(player: &Arc<Player>) -> (Registration, mpsc::UnboundedReceiver<Step>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let id = player.entity_id();
    GATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id, sender);
    (Registration(id), receiver)
}

pub async fn pause(player: &Arc<Player>, phase: Phase) {
    let sender = GATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&player.entity_id())
        .cloned();
    if let Some(sender) = sender {
        let (release, receiver) = oneshot::channel();
        if sender.send(Step { phase, release }).is_ok() {
            assert!(
                tokio::time::timeout(Duration::from_secs(60), receiver)
                    .await
                    .is_ok(),
                "respawn owner-review gate was not released"
            );
        }
    }
}
