use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, stream::SplitSink};
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::warn;

use crate::server::Server;

// Notifications use the hub's bounded, drop-oldest broadcast queue. Replies and
// pongs have separate capacity so a notification flood cannot consume it.
pub(super) const REPLY_CAPACITY: usize = 32;
pub(super) type Outbound = (Message, Option<oneshot::Sender<()>>);

pub(super) async fn run(
    mut sender: SplitSink<WebSocket, Message>,
    server: Arc<Server>,
    mut notifications: broadcast::Receiver<String>,
    mut replies: mpsc::Receiver<Outbound>,
) {
    'write: {
        loop {
            tokio::select! {
                biased;
                () = server.management_shutdown.cancelled() => break,
                reply = replies.recv() => {
                    let Some(reply) = reply else { break };
                    if !send_reply(&mut sender, reply).await {
                        break 'write;
                    }
                }
                notification = notifications.recv() => {
                    match notification {
                        Ok(notification) => {
                            if sender.send(Message::from(notification)).await.is_err() {
                                break 'write;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!("Management client lagged behind by {skipped} messages");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
        if server.management_shutdown.is_cancelled() {
            // The reader closes this queue after any in-flight stop acknowledgement.
            while let Some(reply) = replies.recv().await {
                if !send_reply(&mut sender, reply).await {
                    break 'write;
                }
            }
            // Receiver::len includes overwritten messages. Bound attempts to retained
            // capacity plus one initial lag report, even if producers keep broadcasting.
            for _ in 0..=super::hub::NOTIFICATION_CAPACITY {
                match notifications.try_recv() {
                    Ok(notification) => {
                        if sender.send(Message::from(notification)).await.is_err() {
                            break 'write;
                        }
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        }
    }
    let _ = sender.close().await;
}

async fn send_reply(sender: &mut SplitSink<WebSocket, Message>, reply: Outbound) -> bool {
    let (message, flushed) = reply;
    if sender.send(message).await.is_err() {
        return false;
    }
    if let Some(flushed) = flushed {
        let _ = flushed.send(());
    }
    true
}
