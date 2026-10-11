//! Task-local barriers for the watcher tasks scheduled by a real player teleport.
use pumpkin_util::math::vector2::Vector2;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, PartialEq, Eq)]
pub enum Phase {
    Watched,
    Unwatched,
}

pub struct Step {
    pub phase: Phase,
    pub chunks: Vec<Vector2<i32>>,
    pub release: oneshot::Sender<()>,
}

tokio::task_local! {
    static STEPS: mpsc::UnboundedSender<Step>;
}

/// Captures only watcher tasks created synchronously by this operation.
pub fn capture(operation: impl FnOnce()) -> mpsc::UnboundedReceiver<Step> {
    let (sender, receiver) = mpsc::unbounded_channel();
    STEPS.sync_scope(sender, operation);
    receiver
}

pub(super) fn current() -> Option<mpsc::UnboundedSender<Step>> {
    STEPS.try_with(Clone::clone).ok()
}

/// Pauses without holding locks; dropping a step or receiver also releases the task on unwind.
pub(super) async fn pause(
    sender: Option<&mpsc::UnboundedSender<Step>>,
    phase: Phase,
    chunks: &[Vector2<i32>],
) {
    if let Some(sender) = sender {
        let (release, receiver) = oneshot::channel();
        if sender
            .send(Step {
                phase,
                chunks: chunks.to_vec(),
                release,
            })
            .is_ok()
        {
            let _ = receiver.await;
        }
    }
}
