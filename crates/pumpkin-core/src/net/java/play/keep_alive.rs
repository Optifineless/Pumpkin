#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::ServerPacket;

impl JavaClient {
    pub(in crate::net::java) fn handle_received_keep_alive(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: &pumpkin_protocol::RawPacket,
    ) -> Result<bool, pumpkin_protocol::ser::ReadingError> {
        use pumpkin_protocol::packet::MultiVersionJavaPacket;
        if packet.id != SKeepAlive::to_id(self.version.load()) {
            return Ok(false);
        }
        let mut event = crate::plugin::server::packet::PacketReceivedEvent::new(
            player.clone(),
            packet.id,
            packet.payload.clone(),
        );
        server.plugin_manager.fire_blocking(server, &mut event);
        if event.cancelled || self.is_closed() {
            return Ok(true);
        }
        if event.packet_id != SKeepAlive::to_id(CURRENT_MC_VERSION) {
            return Err(pumpkin_protocol::ser::ReadingError::Message(
                "Invalid translated keep-alive".into(),
            ));
        }
        let mut payload = &event.payload[..];
        let reply = SKeepAlive::read_bounded(&mut payload, &CURRENT_MC_VERSION)?;
        if !payload.is_empty() {
            return Err(pumpkin_protocol::ser::ReadingError::Message(
                "Trailing keep-alive data".into(),
            ));
        }
        self.handle_keep_alive(player, &reply);
        Ok(true)
    }

    pub fn handle_keep_alive(&self, player: &Player, keep_alive: &SKeepAlive) {
        // ServerCommonPacketListenerImpl.handleKeepAlive rejects even duplicate replies.
        let latency = self
            .keep_alive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .acknowledge(keep_alive.keep_alive_id, std::time::Instant::now());
        if let Some(ping) = latency {
            player.ping.store(
                ((u64::from(player.ping.load(Ordering::Relaxed)) * 3 + u64::from(ping)) / 4) as u32,
                Ordering::Relaxed,
            );
        } else {
            self.try_kick(&pumpkin_macros::translate_cross!(
                translation::java::DISCONNECT_TIMEOUT,
                translation::bedrock::DISCONNECT_TIMEOUT
            ));
        }
    }
}
