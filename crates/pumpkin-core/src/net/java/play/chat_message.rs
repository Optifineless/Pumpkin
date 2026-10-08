#[allow(clippy::wildcard_imports)]
use super::*;
use pumpkin_data::world::RAW;

impl JavaClient {
    pub async fn handle_chat_message(
        &self,
        server: &Arc<Server>,
        player: &Arc<Player>,
        chat_message: SChatMessage<'_>,
    ) {
        if self.is_closed() {
            return;
        }
        let gameprofile = &player.gameprofile;

        let Ok(seen) = Self::apply_last_seen(
            player,
            chat_message.message_count.0,
            chat_message.acknowledged,
            chat_message.checksum,
        ) else {
            self.kick(TextComponent::translate(
                "multiplayer.disconnect.chat_validation_failed",
                [],
            ))
            .await;
            return;
        };
        if !self
            .try_handle_chat(player, chat_message.message, false)
            .await
        {
            return;
        }
        let signed = if server.basic_config.allow_chat_reports {
            let result = self.get_signed_message(player, &chat_message, seen);
            match result {
                Ok(message) => Some(message),
                Err(reason) => {
                    player.send_system_message(&TextComponent::translate(reason, []));
                    return;
                }
            }
        } else {
            None
        };
        send_cancellable! {{
            server;
            PlayerChatEvent::new(
                player.clone(),
                chat_message.message.to_string(),
                vec![],
                chat_message.signature.map(<[u8]>::to_vec),
            );

            'after: {
                if self.is_closed() { return; }
                info!("<chat> {}: {}", gameprofile.name, event.message);

                let config = &server.advanced_config;

                let message = match seasonal_events::modify_chat_message(&event.message, config) {
                    Some(m) => m,
                    None => event.message.clone(),
                };

                let decorated_message = TextComponent::chat_decorated(
                    &config.chat.format,
                    &gameprofile.name,
                    &message,
                );

                let entity = &player.get_entity();
                let world = entity.world.load_full();
                if let Some(signed) = signed {
                    Self::broadcast_verified_chat(player, &signed.with_unsigned_content(decorated_message));
                } else {
                    let outgoing = crate::net::chat::PlayerChatMessage::system(message).with_unsigned_content(decorated_message);
                    world.broadcast_chat_message(
                        &outgoing,
                        Player::is_text_filtering_enabled,
                        Some(player),
                        (RAW + 1).into(),
                        &TextComponent::empty(),
                        None,
                    );
                }
            }
        }}
        self.check_session_spam(player, server, crate::entity::player::SpamType::Chat);
    }

    // ServerGamePacketListenerImpl.getSignedMessage uses the per-session decoder.
    fn get_signed_message(
        &self,
        player: &Player,
        message: &SChatMessage<'_>,
        seen: Vec<Box<[u8]>>,
    ) -> Result<crate::net::chat::PlayerChatMessage, &'static str> {
        let session = player
            .chat_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body = crate::net::chat::SignedMessageBody::new(
            message.message.into(),
            message.timestamp,
            message.salt,
            seen,
        );
        self.chat_order
            .chain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unpack(player.gameprofile.id, &session, body, message.signature)
    }

    pub async fn handle_chat_session_update(
        &self,
        player: &Arc<Player>,
        server: &Server,
        session: SPlayerSession,
    ) {
        // Keep the chat session default if we don't want reports
        if self.is_closed() || !server.basic_config.allow_chat_reports {
            return;
        }

        if let Err(err) = self.validate_chat_session(player, server, &session).await {
            log_at_level!(
                err.severity(),
                "{} (uuid {}) {}",
                player.gameprofile.name,
                player.gameprofile.id,
                err
            );
            if err.is_kick()
                && let Some(reason) = err.client_kick_reason()
            {
                self.kick(TextComponent::text(reason)).await;
            }
            return;
        }

        if self.is_closed() {
            return;
        }
        let new_session = ChatSession::new(
            session.session_id,
            session.expires_at,
            session.public_key.clone(),
            session.key_signature.clone(),
        );
        let mut previous = player
            .chat_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if previous.public_key == new_session.public_key
            && previous.expires_at == new_session.expires_at
        {
            return;
        }
        if new_session.expires_at < previous.expires_at {
            drop(previous);
            self.try_kick(&TextComponent::translate(
                "multiplayer.disconnect.expired_public_key",
                [],
            ));
            return;
        }
        if self
            .chat_order
            .chain
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reset(&new_session)
            .is_err()
        {
            drop(previous);
            self.try_kick(&TextComponent::translate(
                "multiplayer.disconnect.invalid_public_key_signature",
                [],
            ));
            return;
        }
        *previous = new_session;
        drop(previous);

        server.broadcast_packet_all(&CPlayerInfoUpdate::new(
            PlayerInfoFlags::INITIALIZE_CHAT.bits(),
            &[pumpkin_protocol::java::client::play::Player {
                uuid: player.gameprofile.id,
                actions: &[PlayerAction::InitializeChat(Some(InitChat {
                    session_id: session.session_id,
                    expires_at: session.expires_at,
                    public_key: session.public_key.clone(),
                    signature: session.key_signature.clone(),
                }))],
            }],
        ));
    }

    /// Runs vanilla checks for a valid player session
    pub async fn validate_chat_session(
        &self,
        player: &Player,
        server: &Server,
        session: &SPlayerSession,
    ) -> Result<(), ChatError> {
        // Verify session expiry
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        if session.expires_at < now {
            return Err(ChatError::InvalidPublicKey);
        }

        let key_signature = RsaPkcs1v15Signature::try_from(session.key_signature.as_ref())
            .map_err(|_| ChatError::InvalidPublicKey)?;

        let mut signable = Vec::new();
        signable.extend_from_slice(player.gameprofile.id.as_bytes());
        signable.extend_from_slice(&session.expires_at.to_be_bytes());
        signable.extend_from_slice(&session.public_key);

        let public_keys = server.mojang_public_keys.load_full();

        let (tx, rx) = tokio::sync::oneshot::channel();
        rayon::spawn(move || {
            let is_valid = public_keys.iter().any(|key| {
                let verifying_key = VerifyingKey::<Sha1>::new(key.clone());
                verifying_key.verify(&signable, &key_signature).is_ok()
            });
            let _ = tx.send(is_valid);
        });
        let is_valid = rx.await.unwrap_or(false);

        // Verify that the signable is valid for any one of Mojang's public keys
        if !is_valid {
            return Err(ChatError::InvalidPublicKey);
        }

        Ok(())
    }
}
