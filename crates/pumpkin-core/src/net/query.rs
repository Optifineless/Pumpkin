use std::{
    ffi::{CString, NulError},
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
};

use pumpkin_protocol::query::{
    CBasicStatus, CFullStatus, CHandshake, PacketType, RawQueryPacket, SHandshake, SStatusRequest,
};
use pumpkin_util::text::{TextComponent, color::NamedColor};
use pumpkin_world::CURRENT_MC_VERSION;
use tokio::net::UdpSocket;
use tracing::{error, info};

use crate::{SHOULD_STOP, STOP_INTERRUPT, server::Server};

const PLUGIN_METADATA_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

pub async fn start_query_handler(server: Arc<Server>, query_addr: SocketAddr) {
    let Ok(socket) = UdpSocket::bind(query_addr).await else {
        error!("Unable to bind query UDP socket");
        return;
    };

    // Challenge tokens are bound to the IP address and port
    let challenges = super::query_challenge::QueryChallenges::new();
    let mut plugins = server
        .plugin_manager
        .try_active_plugin_names()
        .unwrap_or_default()
        .join(", ");
    let mut plugins_refreshed = std::time::Instant::now();

    if let Ok(local_addr) = socket.local_addr() {
        info!(
            "Server query running on port {}",
            TextComponent::text(format!("{}", local_addr.port()))
                .color_named(NamedColor::DarkBlue)
                .to_pretty_console()
        );
    }

    // QueryThreadGs4.run processes datagrams inline.
    // Reused across packets. Handling is done inline so a flood of datagrams
    // cannot pile up unbounded tasks, each owning a buffer of its own.
    let mut buf = vec![0; 1024];

    while !SHOULD_STOP.load(Ordering::Relaxed) {
        let recv_result = tokio::select! {
            result = socket.recv_from(&mut buf) => Some(result),
            () = STOP_INTERRUPT.cancelled() => None,
        };

        let Some(Ok((length, addr))) = recv_result else {
            break;
        };
        // QueryThreadGs4.buildRuleResponse includes plugin names; metadata refresh never waits.
        if plugins_refreshed.elapsed() >= PLUGIN_METADATA_REFRESH_INTERVAL {
            if let Some(names) = server.plugin_manager.try_active_plugin_names() {
                plugins = names.join(", ");
            }
            plugins_refreshed = std::time::Instant::now();
        }

        let result = tokio::select! {
            () = STOP_INTERRUPT.cancelled() => break,
            result = handle_packet(buf[..length].to_vec(), &challenges, &plugins,
                &server, &socket, addr, query_addr) => result,
        };
        if let Err(err) = result {
            error!("Interior 0 bytes found! Cannot encode query response! {err}");
        }
    }
}

// Errors of packets that don't meet the format aren't returned since we won't handle them anyway
// The only errors that are thrown are because of a null terminator in a CString
// since those errors need to be corrected by server owner
#[inline]
async fn handle_packet(
    buf: Vec<u8>,
    challenges: &super::query_challenge::QueryChallenges,
    plugins: &str,
    server: &Server,
    socket: &UdpSocket,
    addr: SocketAddr,
    bound_addr: SocketAddr,
) -> Result<(), NulError> {
    if let Ok(mut raw_packet) = RawQueryPacket::decode(buf).await {
        match raw_packet.packet_type {
            PacketType::Handshake => {
                if let Ok(packet) = SHandshake::decode(&mut raw_packet).await {
                    let challenge_token = challenges.issue(addr);
                    let response = CHandshake {
                        session_id: packet.session_id,
                        challenge_token,
                    };

                    // Ignore all errors since we don't want the query handler to crash
                    // Protocol also ignores all errors and just doesn't respond
                    if let Some(encoded) = response.encode() {
                        let _ = socket.try_send_to(encoded.as_slice(), addr);
                    }
                }
            }
            PacketType::Status => {
                if let Ok(packet) = SStatusRequest::decode(&mut raw_packet).await
                    && challenges.accepts(addr, packet.challenge_token)
                {
                    if packet.is_full_request {
                        // Get 4 players
                        let mut players: Vec<CString> = Vec::new();
                        for world in server.worlds.load().iter() {
                            let mut world_players = world
                                .players
                                .load()
                                // Although there is no documented limit, we will limit to 4 players
                                .iter()
                                .take(4 - players.len())
                                .filter_map(|player| {
                                    CString::new(player.gameprofile.name.as_str()).ok()
                                })
                                .collect::<Vec<_>>();

                            players.append(&mut world_players); // Append players from this world

                            if players.len() >= 4 {
                                break; // Stop if we've collected 4 players
                            }
                        }

                        let response = CFullStatus {
                            session_id: packet.session_id,
                            hostname: CString::new(
                                server.advanced_config.networking.java.motd.as_str(),
                            )?,
                            version: CString::new(CURRENT_MC_VERSION)?,
                            plugins: CString::new(plugins)?,
                            map: CString::new(
                                server
                                    .worlds
                                    .load()
                                    .first()
                                    .map_or("world", |w| w.get_world_name()),
                            )?,
                            num_players: server.get_player_count(),
                            max_players: server.advanced_config.networking.java.max_players
                                as usize,
                            host_port: bound_addr.port(),
                            host_ip: CString::new(bound_addr.ip().to_string())?,
                            players,
                        };

                        if let Some(encoded) = response.encode() {
                            let _ = socket.try_send_to(encoded.as_slice(), addr);
                        }
                    } else {
                        let response = CBasicStatus {
                            session_id: packet.session_id,
                            motd: CString::new(
                                server.advanced_config.networking.java.motd.as_str(),
                            )?,
                            map: CString::new(
                                server
                                    .worlds
                                    .load()
                                    .first()
                                    .map_or("world", |w| w.get_world_name()),
                            )?,
                            num_players: server.get_player_count(),
                            max_players: server.advanced_config.networking.java.max_players
                                as usize,
                            host_port: bound_addr.port(),
                            host_ip: CString::new(bound_addr.ip().to_string())?,
                        };

                        if let Some(encoded) = response.encode() {
                            let _ = socket.try_send_to(encoded.as_slice(), addr);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
