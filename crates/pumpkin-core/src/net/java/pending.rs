use std::{
    net::SocketAddr,
    num::NonZero,
    sync::{Arc, Weak},
};

use bytes::Bytes;
use crossbeam::atomic::AtomicCell;
use pumpkin_config::networking::compression::CompressionInfo;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{
    ClientPacket, ConnectionState, PacketDecodeError, RawPacket, ServerPacket,
    codec::var_int::VarInt,
    java::{
        client::config::CConfigDisconnect,
        client::login::CLoginDisconnect,
        client::play::CPlayDisconnect,
        packet_decoder::TCPNetworkDecoder,
        packet_encoder::TCPNetworkEncoder,
        server::config::{
            SAcceptCodeOfConduct, SAcknowledgeFinishConfig, SClientInformationConfig,
            SConfigCookieResponse, SConfigPong, SConfigResourcePack, SKnownPacks, SPluginMessage,
        },
    },
    packet::MultiVersionJavaPacket,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError},
};
use pumpkin_util::{Hand, text::TextComponent, version::JavaMinecraftVersion};
use tokio::{
    io::{BufReader, BufWriter},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::{
    entity::player::ChatMode,
    net::{
        EncryptionError, GameProfile, PacketHandlerResult, PacketRateLimiter, PlayerConfig,
        can_not_join,
    },
    plugin::server::packet::{ConnectionPacketReceivedEvent, ConnectionPacketSentEvent},
    server::Server,
};

use super::JavaClient;

const BRAND_CHANNEL_PREFIX: &str = "minecraft:brand";

fn handle_outgoing_cancellation(
    close: &CancellationToken,
    state: ConnectionState,
    packet_id: i32,
) -> bool {
    use pumpkin_protocol::java::client::{
        config::{CFinishConfig, CKnownPacks},
        login::{CEncryptionRequest, CLoginSuccess, CSetCompression},
    };

    // ServerLoginPacketListenerImpl.handleHello/verifyLoginAndFinishConnectionSetup/
    // finishLoginAndWaitForClient, SynchronizeRegistriesTask.start and JoinWorldTask.start.
    let required = match state {
        ConnectionState::Login | ConnectionState::Transfer => [
            CEncryptionRequest::to_id(CURRENT_MC_VERSION),
            CSetCompression::to_id(CURRENT_MC_VERSION),
            CLoginSuccess::to_id(CURRENT_MC_VERSION),
        ]
        .contains(&packet_id),
        ConnectionState::Config => [
            CKnownPacks::to_id(CURRENT_MC_VERSION),
            CFinishConfig::to_id(CURRENT_MC_VERSION),
        ]
        .contains(&packet_id),
        _ => false,
    };
    if required {
        close.cancel();
    }
    !required
}

pub struct PendingConnection {
    pub id: u64,
    pub address: SocketAddr,
    pub server_address: String,
    pub version: AtomicCell<JavaMinecraftVersion>,
    pub connection_state: AtomicCell<ConnectionState>,
    pub close_token: CancellationToken,
    pub network_writer: TCPNetworkEncoder<BufWriter<OwnedWriteHalf>>,
    pub network_reader: TCPNetworkDecoder<BufReader<OwnedReadHalf>>,
    pub gameprofile: Option<GameProfile>,
    pub requested_username: Option<String>,
    pub login_state: super::login::state::LoginState,
    pub config: Option<PlayerConfig>,
    pub(super) keep_alive: super::session::KeepAliveState,
    pub(super) config_task: super::config::ConfigTask,
    pub(super) login_deadline_active: Arc<std::sync::atomic::AtomicBool>,
    pub brand: Option<String>,
    pub packet_limiter: PacketRateLimiter,
    pub verify_token: Option<[u8; 4]>,
    pub vine_challenge: Option<[u8; 16]>,
    /// For the connection packet events.
    server: Weak<Server>,
}

impl PendingConnection {
    #[must_use]
    pub fn new(
        tcp_stream: TcpStream,
        address: SocketAddr,
        id: u64,
        packet_limiter: PacketRateLimiter,
        server: Weak<Server>,
    ) -> Self {
        let (read, write) = tcp_stream.into_split();
        Self {
            id,
            address,
            server_address: String::new(),
            version: AtomicCell::new(CURRENT_MC_VERSION),
            connection_state: AtomicCell::new(ConnectionState::HandShake),
            close_token: crate::STOP_INTERRUPT.child_token(),
            network_writer: TCPNetworkEncoder::new(BufWriter::new(write)),
            network_reader: TCPNetworkDecoder::new(BufReader::new(read)),
            gameprofile: None,
            requested_username: None,
            login_state: super::login::state::LoginState::Hello,
            config: None,
            keep_alive: super::session::KeepAliveState::new(std::time::Instant::now()),
            config_task: super::config::ConfigTask::NotStarted,
            login_deadline_active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            brand: None,
            packet_limiter,
            verify_token: None,
            vine_challenge: None,
            server,
        }
    }

    pub fn close(&self) {
        self.close_token.cancel();
    }

    pub fn is_closed(&self) -> bool {
        self.close_token.is_cancelled()
    }

    pub async fn await_close_interrupt(&self) {
        self.close_token.cancelled().await;
    }

    pub fn set_encryption(&mut self, shared_secret: &[u8]) -> Result<(), EncryptionError> {
        let crypt_key: [u8; 16] = shared_secret
            .try_into()
            .map_err(|_| EncryptionError::SharedWrongLength)?;
        self.network_reader
            .set_encryption(&crypt_key)
            .map_err(|_| EncryptionError::AlreadyEncrypted)?;
        self.network_writer
            .set_encryption(&crypt_key)
            .map_err(|_| EncryptionError::AlreadyEncrypted)?;
        Ok(())
    }

    pub fn set_compression(&mut self, compression: &CompressionInfo) {
        if compression.level > 9 {
            error!("Invalid compression level! Clients will not be able to read this!");
        }

        self.network_reader
            .set_compression(compression.threshold as usize);

        self.network_writer
            .set_compression((compression.threshold as usize, compression.level));
    }

    pub async fn get_packet(&mut self) -> Option<RawPacket> {
        let close_token = self.close_token.clone();
        let mut timer = tokio::time::interval(std::time::Duration::from_millis(50));
        let packet_result = loop {
            tokio::select! {
                biased;
                () = close_token.cancelled() => return None,
                res = self.network_reader.get_raw_packet() => break res,
                _ = timer.tick(), if self.connection_state.load() == ConnectionState::Config => {
                    // The decoder retains partial frame state across this cancellation.
                    if !self.keep_config_connection_alive().await { return None; }
                }
            }
        };

        match packet_result {
            Ok(packet) => Some(packet),
            Err(err) => {
                if !matches!(err, PacketDecodeError::ConnectionClosed) {
                    debug!("Failed to decode packet from client {}: {}", self.id, err);
                    let reason = if matches!(err, PacketDecodeError::ReadTimeout) {
                        TextComponent::translate("disconnect.timeout", [])
                    } else {
                        TextComponent::text(format!("Error while reading incoming packet {err}"))
                    };
                    self.kick(reason).await;
                }
                None
            }
        }
    }

    /// Server for the connection packet events, only for clients below 26.3 after handshake.
    fn translating_server(&self) -> Option<Arc<Server>> {
        if self.version.load() == CURRENT_MC_VERSION
            || self.connection_state.load() == ConnectionState::HandShake
        {
            return None;
        }
        self.server.upgrade()
    }

    /// Encoded as 26.3. `ConnectionPacketSentEvent` can rewrite it.
    /// Returns true after writing and flushing, or silently dropping a cancelled optional packet.
    pub async fn send_packet_now<P: ClientPacket>(&mut self, packet: &P) -> bool {
        let close = self.close_token.clone();
        let write = async {
            let mut packet_buf = Vec::new();
            JavaClient::write_packet_for_version(packet, CURRENT_MC_VERSION, &mut packet_buf)
                .map_err(|err| err.to_string())?;
            let Some(payload) = self.translate_outgoing(Bytes::from(packet_buf)).await else {
                return Ok(false);
            };
            self.network_writer
                .write_packet(payload)
                .await
                .map_err(|err| err.to_string())?;
            self.network_writer
                .flush()
                .await
                .map_err(|err| err.to_string())?;
            Ok(true)
        };
        // Pre-play writes must release connection tasks on shutdown or a silent peer.
        let result: Option<Result<bool, String>> =
            super::session::await_pending_write(&close, write).await;
        match result {
            Some(Ok(sent)) => {
                sent || handle_outgoing_cancellation(
                    &close,
                    self.connection_state.load(),
                    P::to_id(CURRENT_MC_VERSION),
                )
            }
            Some(Err(err)) => {
                debug!("Failed to send packet to client {}: {err}", self.id);
                self.close();
                false
            }
            None => {
                self.close();
                false
            }
        }
    }

    /// `ConnectionPacketSentEvent` with the 26.3 packet. `None` when cancelled.
    async fn translate_outgoing(&self, packet_data: Bytes) -> Option<Bytes> {
        let Some(server) = self.translating_server() else {
            return Some(packet_data);
        };
        if !server
            .plugin_manager
            .has_handlers::<ConnectionPacketSentEvent>()
        {
            return Some(packet_data);
        }

        let mut reader = &packet_data[..];
        let Ok(packet_id) = reader.get_var_int() else {
            return Some(packet_data);
        };
        let payload = packet_data.slice(packet_data.len() - reader.len()..);
        let mut event = ConnectionPacketSentEvent::new(
            self.id,
            self.version.load(),
            self.connection_state.load(),
            packet_id.0,
            payload,
        );
        server.plugin_manager.fire(&server, &mut event).await;
        if event.cancelled {
            return None;
        }

        let mut framed = Vec::with_capacity(5 + event.payload.len());
        framed.write_var_int(&VarInt(event.packet_id)).ok()?;
        framed.extend_from_slice(&event.payload);
        Some(framed.into())
    }

    /// `ConnectionPacketReceivedEvent` with the client's packet; handlers rewrite it to 26.3.
    /// `None` when cancelled.
    async fn translate_incoming(&self, packet: &RawPacket) -> Option<RawPacket> {
        let unchanged = || RawPacket {
            id: packet.id,
            payload: packet.payload.clone(),
        };
        let Some(server) = self.translating_server() else {
            return Some(unchanged());
        };
        if !server
            .plugin_manager
            .has_handlers::<ConnectionPacketReceivedEvent>()
        {
            return Some(unchanged());
        }

        let mut event = ConnectionPacketReceivedEvent::new(
            self.id,
            self.version.load(),
            self.connection_state.load(),
            packet.id,
            packet.payload.clone(),
        );
        server.plugin_manager.fire(&server, &mut event).await;
        (!event.cancelled).then(|| RawPacket {
            id: event.packet_id,
            payload: event.payload,
        })
    }

    pub async fn kick(&mut self, reason: TextComponent) {
        match self.connection_state.load() {
            ConnectionState::Login => {
                self.send_packet_now(&CLoginDisconnect::new(
                    serde_json::to_string(&reason.0).unwrap_or_else(|_| String::new()),
                ))
                .await;
            }
            ConnectionState::Config => {
                self.send_packet_now(&CConfigDisconnect::new(&reason.get_text()))
                    .await;
            }
            ConnectionState::Play => {
                self.send_packet_now(&CPlayDisconnect::new(&reason)).await;
            }
            _ => {}
        }
        debug!("Closing connection for {}", self.id);
        self.close();
    }

    pub async fn handle_login_sequence(&mut self, server: &Arc<Server>) -> PacketHandlerResult {
        use std::sync::atomic::Ordering;
        let started = server.tick_count.load(Ordering::Relaxed);
        let active = self.login_deadline_active.clone();
        let close = self.close_token.clone();
        let result = {
            let run = self.run_login_sequence(server);
            tokio::pin!(run);
            let mut timer = tokio::time::interval(std::time::Duration::from_millis(50));
            loop {
                tokio::select! {
                    () = close.cancelled() => return PacketHandlerResult::Stop,
                    _ = timer.tick() => {
                        // ServerLoginPacketListenerImpl.tick: 600 server ticks total.
                        if active.load(Ordering::Acquire)
                            && server.tick_count.load(Ordering::Relaxed).wrapping_sub(started)
                                >= super::session::MAX_TICKS_BEFORE_LOGIN
                        { break None; }
                    }
                    result = &mut run => break Some(result),
                }
            }
        };
        if let Some(result) = result {
            return result;
        }
        self.kick(TextComponent::translate(
            "multiplayer.disconnect.slow_login",
            [],
        ))
        .await;
        PacketHandlerResult::Stop
    }

    async fn run_login_sequence(&mut self, server: &Arc<Server>) -> PacketHandlerResult {
        while let Some(packet) = self.get_packet().await {
            if !self.packet_limiter.check_packet() {
                warn!(
                    "Pending client {} exceeded packet rate limit (rate: {}/s)",
                    self.id,
                    self.packet_limiter.max_rate()
                );
                self.kick(TextComponent::text(
                    server
                        .advanced_config
                        .networking
                        .java
                        .packet_limiter
                        .kick_message
                        .clone(),
                ))
                .await;
                return PacketHandlerResult::Stop;
            }

            match self.handle_packet(server, &packet).await {
                Ok(result) => {
                    if let Some(result) = result {
                        return result;
                    }
                }
                Err(error) => {
                    let text = format!("Error while reading incoming packet {error}");
                    debug!(
                        "Failed to read incoming packet with id {}: {}",
                        packet.id, error
                    );
                    self.kick(TextComponent::text(text)).await;
                }
            }
        }
        PacketHandlerResult::Stop
    }

    pub async fn handle_packet(
        &mut self,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<Option<PacketHandlerResult>, ReadingError> {
        let Some(packet) = self.translate_incoming(packet).await else {
            return Ok(None);
        };
        let packet = &packet;
        match self.connection_state.load() {
            ConnectionState::HandShake => self.handle_handshake_packet(server, packet).await,
            ConnectionState::Status => self.handle_status_packet(server, packet).await,
            ConnectionState::Login | ConnectionState::Transfer => {
                self.handle_login_packet(server, packet).await
            }
            ConnectionState::Config => self.handle_config_packet(server, packet).await,
            ConnectionState::Play => Ok(None),
        }
    }

    async fn handle_handshake_packet(
        &mut self,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<Option<PacketHandlerResult>, ReadingError> {
        debug!("Handling handshake group");
        let mut payload = &packet.payload[..];
        match packet.id {
            0 => {
                self.handle_handshake(
                    server,
                    pumpkin_protocol::java::server::handshake::SHandShake::read_bounded(
                        &mut payload,
                        &CURRENT_MC_VERSION,
                    )?,
                )
                .await;
                Ok(None)
            }
            _ => Err(ReadingError::Message(format!(
                "Failed to handle packet id {} in Handshake State",
                packet.id
            ))),
        }
    }

    async fn handle_status_packet(
        &mut self,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<Option<PacketHandlerResult>, ReadingError> {
        debug!("Handling status group");
        let mut payload = &packet.payload[..];
        let version = CURRENT_MC_VERSION;

        match packet.id {
            id if id == pumpkin_protocol::java::server::status::SStatusRequest::to_id(version) => {
                self.handle_status_request(server).await;
                Ok(None)
            }
            id if id
                == pumpkin_protocol::java::server::status::SStatusPingRequest::to_id(version) =>
            {
                self.handle_ping_request(
                    pumpkin_protocol::java::server::status::SStatusPingRequest::read_bounded(
                        &mut payload,
                        &version,
                    )?,
                )
                .await;
                Ok(None)
            }
            _ => Err(ReadingError::Message(format!(
                "Failed to handle java client packet id {} in Status State",
                packet.id
            ))),
        }
    }

    async fn handle_login_packet(
        &mut self,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<Option<PacketHandlerResult>, ReadingError> {
        debug!("Handling login group");
        let mut payload = &packet.payload[..];
        let version = CURRENT_MC_VERSION;

        match packet.id {
            id if id == pumpkin_protocol::java::server::login::SLoginStart::to_id(version) => {
                Ok(self
                    .handle_login_start(
                        server,
                        pumpkin_protocol::java::server::login::SLoginStart::read_bounded(
                            &mut payload,
                            &version,
                        )?,
                    )
                    .await)
            }
            id if id
                == pumpkin_protocol::java::server::login::SEncryptionResponse::to_id(version) =>
            {
                Ok(self
                    .handle_encryption_response(
                        server,
                        pumpkin_protocol::java::server::login::SEncryptionResponse::read_bounded(
                            &mut payload,
                            &version,
                        )?,
                    )
                    .await)
            }
            id if id
                == pumpkin_protocol::java::server::login::SLoginPluginResponse::to_id(version) =>
            {
                Ok(self
                    .handle_plugin_response(
                        server,
                        pumpkin_protocol::java::server::login::SLoginPluginResponse::read_bounded(
                            &mut payload,
                            &version,
                        )?,
                    )
                    .await)
            }
            id if id
                == pumpkin_protocol::java::server::login::SLoginCookieResponse::to_id(version) =>
            {
                self.handle_login_cookie_response(
                    &pumpkin_protocol::java::server::login::SLoginCookieResponse::read_bounded(
                        &mut payload,
                        &version,
                    )?,
                )
                .await;
                Ok(None)
            }
            id if id
                == pumpkin_protocol::java::server::login::SLoginAcknowledged::to_id(version) =>
            {
                Ok(self.handle_login_acknowledged(server).await)
            }
            _ => Err(ReadingError::Message(format!(
                "Failed to handle packet id {} in Login State",
                packet.id
            ))),
        }
    }

    async fn handle_config_packet(
        &mut self,
        server: &Arc<Server>,
        packet: &RawPacket,
    ) -> Result<Option<PacketHandlerResult>, ReadingError> {
        debug!("Handling config group");
        let mut payload = &packet.payload[..];
        let version = CURRENT_MC_VERSION;

        match packet.id {
            id if id == SClientInformationConfig::to_id(version) => {
                self.handle_client_information_config(SClientInformationConfig::read_bounded(
                    &mut payload,
                    &version,
                )?)
                .await;
                Ok(None)
            }
            id if id == SPluginMessage::to_id(version) => {
                self.handle_plugin_message(SPluginMessage::read_bounded(&mut payload, &version)?)
                    .await;
                Ok(None)
            }
            id if id == SAcknowledgeFinishConfig::to_id(version) => {
                SAcknowledgeFinishConfig::read_bounded(&mut payload, &version)?;
                if !payload.is_empty() {
                    return Err(ReadingError::Message(
                        "Trailing configuration acknowledgement data".into(),
                    ));
                }
                self.config_task
                    .complete(super::config::ConfigTask::JoinWorld)?;
                let Some(profile) = self.gameprofile.clone() else {
                    return Ok(Some(PacketHandlerResult::Stop));
                };
                let config = self.config.clone().unwrap_or_default();
                self.connection_state.store(ConnectionState::Play);
                if let Some(reason) = can_not_join(&profile, &self.address, server) {
                    self.kick(reason).await;
                    Ok(Some(PacketHandlerResult::Stop))
                } else {
                    Ok(Some(PacketHandlerResult::ReadyToPlay(profile, config)))
                }
            }
            id if id == SKnownPacks::to_id(version) => {
                let selection = SKnownPacks::read_bounded(&mut payload, &version)?;
                if !payload.is_empty() {
                    return Err(ReadingError::Message("Trailing known-pack data".into()));
                }
                self.handle_known_packs_response(server, &selection).await?;
                Ok(None)
            }
            id if id == SConfigResourcePack::to_id(version) => {
                self.handle_resource_pack_response(
                    server,
                    SConfigResourcePack::read_bounded(&mut payload, &version)?,
                )
                .await;
                Ok(None)
            }
            id if id == SConfigCookieResponse::to_id(version) => {
                self.handle_config_cookie_response(&SConfigCookieResponse::read_bounded(
                    &mut payload,
                    &version,
                )?)
                .await;
                Ok(None)
            }
            id if id == pumpkin_protocol::java::server::config::SKeepAlive::to_id(version) => {
                let reply = pumpkin_protocol::java::server::config::SKeepAlive::read_bounded(
                    &mut payload,
                    &version,
                )?;
                self.handle_config_keep_alive(reply.keep_alive_id).await;
                Ok(None)
            }
            id if id == SConfigPong::to_id(version) => {
                let _pong = SConfigPong::read_bounded(&mut payload, &version)?;
                Ok(None)
            }
            id if id == SAcceptCodeOfConduct::to_id(version) => {
                SAcceptCodeOfConduct::read_bounded(&mut payload, &version)?;
                Err(ReadingError::Message(
                    "Unexpected code of conduct response".into(),
                ))
            }
            _ => Err(ReadingError::Message(format!(
                "Failed to handle packet id {} in Config State",
                packet.id
            ))),
        }
    }

    pub async fn handle_client_information_config(
        &mut self,
        client_information: SClientInformationConfig<'_>,
    ) {
        debug!("Handling client settings");
        if client_information.view_distance <= 0 {
            self.kick(TextComponent::text(
                "Cannot have zero or negative view distance!",
            ))
            .await;
            return;
        }

        if let (Ok(main_hand), Ok(chat_mode)) = (
            Hand::try_from(client_information.main_hand.0),
            ChatMode::try_from(client_information.chat_mode.0),
        ) {
            self.config = Some(PlayerConfig {
                locale: client_information.locale.to_string(),
                view_distance: NonZero::new(client_information.view_distance as u8)
                    .unwrap_or(NonZero::<u8>::MIN),
                chat_mode,
                chat_colors: client_information.chat_colors,
                skin_parts: client_information.skin_parts,
                main_hand,
                text_filtering: client_information.text_filtering,
                server_listing: client_information.server_listing,
            });
        } else {
            self.kick(TextComponent::text("Invalid hand or chat type"))
                .await;
        }
    }

    pub async fn handle_plugin_message(&mut self, plugin_message: SPluginMessage<'_>) {
        debug!("Handling plugin message");
        if plugin_message.channel.starts_with(BRAND_CHANNEL_PREFIX) {
            debug!("Got a client brand");
            match core::str::from_utf8(plugin_message.data) {
                Ok(brand) => self.brand = Some(brand.to_string()),
                Err(e) => self.kick(TextComponent::text(e.to_string())).await,
            }
        }
    }

    pub async fn handle_config_cookie_response(&mut self, _packet: &SConfigCookieResponse<'_>) {
        // ServerCommonPacketListenerImpl.handleCookieResponse: no request is outstanding.
        self.kick(TextComponent::translate(
            "multiplayer.disconnect.unexpected_query_response",
            [],
        ))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_protocol::java::client::{
        config::{
            CConfigKeepAlive, CConfigServerLinks, CFeatureFlags, CFinishConfig, CKnownPacks,
            CPluginMessage, CUpdateTags,
        },
        login::{CEncryptionRequest, CLoginPluginRequest, CLoginSuccess, CSetCompression},
    };

    #[test]
    fn cancellation_closes_only_state_transition_packets() {
        for (state, id) in [
            (
                ConnectionState::Config,
                CPluginMessage::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CConfigServerLinks::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CUpdateTags::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CConfigKeepAlive::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CFeatureFlags::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Login,
                CLoginPluginRequest::to_id(CURRENT_MC_VERSION),
            ),
        ] {
            let close = CancellationToken::new();
            assert!(handle_outgoing_cancellation(&close, state, id));
            assert!(!close.is_cancelled());
        }
        for (state, id) in [
            (
                ConnectionState::Login,
                CEncryptionRequest::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Login,
                CSetCompression::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Login,
                CLoginSuccess::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CKnownPacks::to_id(CURRENT_MC_VERSION),
            ),
            (
                ConnectionState::Config,
                CFinishConfig::to_id(CURRENT_MC_VERSION),
            ),
        ] {
            let close = CancellationToken::new();
            assert!(!handle_outgoing_cancellation(&close, state, id));
            assert!(close.is_cancelled());
        }
    }
}
