use super::JavaClient;
use crate::{
    entity::player::{Player, SpamType},
    net::ClientPlatform,
    server::Server,
};
use bytes::Bytes;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{
    RawPacket, ServerPacket,
    java::server::play::{
        SChatAck, SChatCommand, SChatCommandSigned, SChatMessage, SPlayerSession,
    },
    packet::MultiVersionJavaPacket,
    ser::ReadingError,
};
use pumpkin_util::{PermissionLvl, text::TextComponent};
use std::sync::{Arc, Mutex, atomic::Ordering};
use tokio::sync::mpsc;

const MAX_PENDING_CHAT_PACKETS: usize = 64;

pub(super) struct ChatOrder {
    sender: mpsc::Sender<RawPacket>,
    receiver: Mutex<Option<mpsc::Receiver<RawPacket>>>,
    spam: Mutex<SpamCounters>,
    pub outgoing: Mutex<super::play::chat_delivery::ChatDeliveryState>,
    pub chain: Mutex<super::play::chat_chain::SignedMessageChain>,
}

#[derive(Default)]
struct SpamCounters {
    tick: i32,
    chat: u32,
    command: u32,
}

impl SpamCounters {
    // TickThrottler.tick/increment/isUnderThreshold; elapsed SERVER ticks, not wall time.
    fn increment(
        &mut self,
        tick: i32,
        command: bool,
        cost: u32,
        decay: u32,
        threshold: u32,
    ) -> bool {
        let ticks = tick.wrapping_sub(self.tick).max(0) as u32;
        self.tick = tick;
        self.chat = self.chat.saturating_sub(ticks.saturating_mul(decay));
        self.command = self.command.saturating_sub(ticks.saturating_mul(decay));
        let counter = if command {
            &mut self.command
        } else {
            &mut self.chat
        };
        *counter = counter.saturating_add(cost);
        threshold > 0 && *counter >= threshold
    }
}

impl ChatOrder {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(MAX_PENDING_CHAT_PACKETS);
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            spam: Mutex::new(SpamCounters::default()),
            outgoing: Mutex::new(super::play::chat_delivery::ChatDeliveryState::default()),
            chain: Mutex::new(super::play::chat_chain::SignedMessageChain::default()),
        }
    }
}

fn validate_packet(id: i32, mut payload: &[u8]) -> Result<(), ReadingError> {
    let v = CURRENT_MC_VERSION;
    match id {
        id if id == SChatAck::to_id(v) => {
            SChatAck::read(&mut payload, &v)?;
        }
        id if id == SChatCommand::to_id(v) => {
            SChatCommand::read(&mut payload, &v)?;
        }
        id if id == SChatCommandSigned::to_id(v) => {
            SChatCommandSigned::read(&mut payload, &v)?;
        }
        id if id == SChatMessage::to_id(v) => {
            SChatMessage::read(&mut payload, &v)?;
        }
        id if id == SPlayerSession::to_id(v) => {
            SPlayerSession::read(&mut payload, &v)?;
        }
        _ => return Err(ReadingError::Message("Unexpected chat packet".into())),
    }
    if !payload.is_empty() {
        return Err(ReadingError::Message("Trailing chat packet data".into()));
    }
    Ok(())
}

impl JavaClient {
    pub(super) fn queue_chat_packet(&self, id: i32, payload: &Bytes) -> Result<(), ReadingError> {
        // Signed commands must never fall back to unsigned decoding.
        validate_packet(id, payload)?;
        self.chat_order
            .sender
            .try_send(RawPacket {
                id,
                payload: payload.clone(),
            })
            .map_err(|_| ReadingError::TooLarge("Pending chat packets".into()))
    }

    pub(super) fn start_chat_worker(&self, player: &Arc<Player>) {
        let Some(mut receiver) = self
            .chat_order
            .receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let player = Arc::downgrade(player);
        let close = self.close_token.clone();
        self.spawn_task(async move {
            loop {
                let packet = tokio::select! {
                    biased;
                    () = close.cancelled() => break,
                    packet = receiver.recv() => packet,
                };
                let Some(packet) = packet else { break; };
                let Some(player) = player.upgrade() else { break; };
                let ClientPlatform::Java(client) = player.client.as_ref() else { break; };
                if client.is_closed() { break; }
                let Some(server) = player.world().server.upgrade() else { break; };
                // ServerGamePacketListenerImpl.chatMessageChain (FutureChain): one FIFO worker.
                tokio::select! {
                    () = close.cancelled() => break,
                    result = client.process_chat_packet(&player, &server, &packet) => {
                        if result.is_err() {
                            client.try_kick(&TextComponent::translate("multiplayer.disconnect.chat_validation_failed", []));
                        }
                    }
                }
            }
        });
    }

    async fn process_chat_packet(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<(), ReadingError> {
        let mut payload = &packet.payload[..];
        let v = CURRENT_MC_VERSION;
        match packet.id {
            id if id == SChatAck::to_id(v) => {
                self.handle_chat_ack(player, &SChatAck::read(&mut payload, &v)?);
            }
            id if id == SChatCommand::to_id(v) => {
                self.handle_chat_command(player, server, &SChatCommand::read(&mut payload, &v)?)
                    .await;
            }
            id if id == SChatCommandSigned::to_id(v) => {
                let signed = SChatCommandSigned::read(&mut payload, &v)?;
                let seen = Self::apply_last_seen(
                    player,
                    signed.message_count.0,
                    signed.acknowledged,
                    signed.checksum,
                )
                .map_err(|_| ReadingError::Message("Invalid command acknowledgements".into()))?;
                self.handle_signed_command(player, server, signed, seen)
                    .await;
            }
            id if id == SChatMessage::to_id(v) => {
                self.handle_chat_message(server, player, SChatMessage::read(&mut payload, &v)?)
                    .await;
            }
            id if id == SPlayerSession::to_id(v) => {
                self.handle_chat_session_update(
                    player,
                    server,
                    SPlayerSession::read(&mut payload, &v)?,
                )
                .await;
            }
            _ => return Err(ReadingError::Message("Unexpected chat packet".into())),
        }
        Ok(())
    }

    pub(super) fn check_session_spam(
        &self,
        player: &Player,
        server: &Server,
        kind: SpamType,
    ) -> bool {
        let config = &server.advanced_config.chat.anti_spam;
        let command = matches!(kind, SpamType::Command);
        let threshold = if command {
            config.command_threshold_ticks()
        } else {
            config.chat_threshold_ticks()
        };
        let exceeded = self
            .chat_order
            .spam
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .increment(
                server.tick_count.load(Ordering::Relaxed),
                command,
                config.message_cost,
                config.decay_per_tick,
                threshold,
            );
        if exceeded
            && config.enabled
            && !(config.ops_bypass && player.permission_lvl.load() > PermissionLvl::Zero)
        {
            self.try_kick(&TextComponent::translate("disconnect.spam", []));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spam_counters_are_separate_and_decay_at_server_tick_rate() {
        let mut counters = SpamCounters::default();
        for _ in 0..9 {
            assert!(!counters.increment(1, false, 20, 1, 200));
            assert!(!counters.increment(1, true, 20, 1, 200));
        }
        assert!(counters.increment(1, false, 20, 1, 200));
        assert!(!counters.increment(22, true, 20, 1, 200));
        assert!(!counters.increment(22, false, 20, 1, 0));
    }

    #[test]
    fn malformed_signed_command_is_rejected() {
        assert!(validate_packet(SChatCommandSigned::to_id(CURRENT_MC_VERSION), &[0]).is_err());
        assert!(validate_packet(SChatCommand::to_id(CURRENT_MC_VERSION), &[0]).is_ok());
    }
}
