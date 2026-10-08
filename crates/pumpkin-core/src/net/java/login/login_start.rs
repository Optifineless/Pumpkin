#[allow(clippy::wildcard_imports)]
use super::*;

impl PendingConnection {
    pub async fn handle_login_start(
        &mut self,
        server: &Arc<Server>,
        login_start: SLoginStart,
    ) -> Option<PacketHandlerResult> {
        debug!("login start");

        // ServerLoginPacketListenerImpl.handleHello rejects repeated starts.
        if self.login_state != super::state::LoginState::Hello {
            self.kick(TextComponent::text("Unexpected login start"))
                .await;
            return Some(PacketHandlerResult::Stop);
        }
        self.login_state = super::state::LoginState::Verifying;
        self.requested_username = Some(login_start.name.to_string());

        let max_players = server.advanced_config.networking.java.max_players;
        if max_players > 0 && server.get_player_count() >= max_players as usize {
            self.kick(TextComponent::translate_cross(
                translation::java::MULTIPLAYER_DISCONNECT_SERVER_FULL,
                translation::bedrock::DISCONNECTIONSCREEN_SERVERFULL,
                [],
            ))
            .await;
            return Some(PacketHandlerResult::Stop);
        }

        if !is_valid_player_name(&login_start.name) {
            self.kick(TextComponent::text("Invalid characters in username"))
                .await;
            return Some(PacketHandlerResult::Stop);
        }

        let proxy = &server.advanced_config.networking.proxy;
        if proxy.enabled {
            self.login_state = super::state::LoginState::Proxy;
            if proxy.vine.enabled {
                vine::vine_login(self).await;
                None
            } else if proxy.velocity.enabled {
                velocity::velocity_login(self).await;
                None
            } else if proxy.bungeecord.enabled {
                match bungeecord::bungeecord_login(
                    &self.address,
                    &self.server_address,
                    login_start.name.into_string(),
                    &proxy.bungeecord.secret,
                ) {
                    Ok((ip, profile)) => {
                        self.address.set_ip(ip);
                        self.gameprofile = Some(profile.clone());
                        self.finish_login(server, &profile).await
                    }
                    Err(error) => {
                        self.kick(TextComponent::text(error.to_string())).await;
                        Some(PacketHandlerResult::Stop)
                    }
                }
            } else {
                None
            }
        } else if server.advanced_config.networking.java.online_mode
            || server.advanced_config.networking.java.encryption
        {
            self.login_state = super::state::LoginState::Key;
            let verify_token: [u8; 4] = rand::random();
            self.verify_token = Some(verify_token);
            self.send_packet_now(
                &server
                    .encryption_request(
                        &verify_token,
                        server.advanced_config.networking.java.online_mode,
                    )
                    .await,
            )
            .await;
            None
        } else {
            let Ok(id) = offline_uuid(&login_start.name) else {
                self.kick(TextComponent::text("Invalid username")).await;
                return Some(PacketHandlerResult::Stop);
            };
            let profile = GameProfile {
                id,
                name: login_start.name.into_string(),
                properties: ArcSwap::new(Arc::new(vec![])),
                profile_actions: None,
            };
            self.finish_login(server, &profile).await
        }
    }
}
