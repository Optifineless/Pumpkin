use super::super::JavaClient;
use crate::{
    entity::player::Player,
    net::{ClientPlatform, chat::PlayerChatMessage},
};
use pumpkin_protocol::{
    VarInt,
    bedrock::server::text::SText,
    java::client::play::{CPlayerChatMessage, PreviousMessage},
};
use pumpkin_util::text::TextComponent;
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub struct ChatDeliveryState {
    index: i32,
    draining: bool,
    queue: VecDeque<Delivery>,
}

struct Delivery {
    recipient: Weak<Player>,
    message: PlayerChatMessage,
    sender_name: TextComponent,
    chat_type: VarInt,
    target_name: Option<TextComponent>,
}

impl JavaClient {
    // ServerPlayer.sendChatMessage / OutgoingChatMessage.Player.sendToPlayer. Full last-seen
    // signatures are valid even when cached; never borrow a cache while looking up that cache.
    pub(super) fn broadcast_verified_chat(sender: &Arc<Player>, message: &PlayerChatMessage) {
        let world = sender.world();
        let sender_name = TextComponent::text(sender.gameprofile.name.clone());
        for recipient in world.players.load().iter() {
            match recipient.client.as_ref() {
                ClientPlatform::Java(client) => {
                    client.send_verified_chat(
                        recipient,
                        message,
                        &sender_name,
                        (pumpkin_data::world::RAW + 1).into(),
                        None,
                    );
                }
                ClientPlatform::Bedrock(client) => client.try_enqueue_client_packet(&SText::new(
                    message.decorated_content().get_text(),
                    sender_name.clone().get_text(),
                )),
            }
        }
    }

    /// Publishes a verified command argument with its bound chat type, committing only sent packets.
    pub(crate) fn send_command_chat(
        recipient: &Arc<Player>,
        message: &PlayerChatMessage,
        chat_type: VarInt,
        sender_name: &TextComponent,
        target_name: Option<&TextComponent>,
    ) {
        match recipient.client.as_ref() {
            ClientPlatform::Java(client) => {
                client.send_verified_chat(recipient, message, sender_name, chat_type, target_name);
            }
            ClientPlatform::Bedrock(client) => client.try_enqueue_client_packet(&SText::new(
                message.decorated_content().get_text(),
                sender_name.clone().get_text(),
            )),
        }
    }

    fn send_verified_chat(
        &self,
        recipient: &Arc<Player>,
        message: &PlayerChatMessage,
        sender_name: &TextComponent,
        chat_type: VarInt,
        target_name: Option<&TextComponent>,
    ) {
        let start = {
            let mut state = self
                .chat_order
                .outgoing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_closed() {
                return;
            }
            if state.queue.len() >= 256 {
                drop(state);
                self.try_kick(&TextComponent::text("Too many pending chats"));
                return;
            }
            state.queue.push_back(Delivery {
                recipient: Arc::downgrade(recipient),
                message: message.clone(),
                sender_name: sender_name.clone(),
                chat_type,
                target_name: target_name.cloned(),
            });
            if state.draining {
                false
            } else {
                state.draining = true;
                true
            }
        };
        if !start {
            return;
        }
        // Reentrant plugin sends append to this queue. No delivery/cache lock surrounds callbacks.
        loop {
            let next = {
                let mut state = self
                    .chat_order
                    .outgoing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(delivery) = state.queue.pop_front() {
                    Some((state.index, delivery))
                } else {
                    state.draining = false;
                    None
                }
            };
            let Some((index, delivery)) = next else {
                break;
            };
            let Some(recipient) = delivery.recipient.upgrade() else {
                continue;
            };
            if self.is_closed() {
                continue;
            }
            self.publish_verified_chat(&recipient, &delivery, index);
        }
    }

    fn publish_verified_chat(&self, recipient: &Player, delivery: &Delivery, index: i32) {
        let message = &delivery.message;
        let last_seen = message
            .signed_body
            .last_seen
            .iter()
            .map(|signature| PreviousMessage {
                id: VarInt(0),
                signature: Some(signature.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let packet = CPlayerChatMessage::new(
            VarInt(index),
            message.sender(),
            VarInt(message.link.index),
            message.signature.clone(),
            message.signed_content().into(),
            message.timestamp(),
            message.salt(),
            last_seen,
            message.unsigned_content.clone(),
            message.filter_mask.to_filter_type(),
            delivery.chat_type,
            delivery.sender_name.clone(),
            delivery.target_name.clone(),
        );
        let Ok(data) = self.serialize_packet(&packet) else {
            return;
        };
        let Some((data, packet_len)) = self.reserve_pending_bytes(data) else {
            return;
        };
        // Translation and PacketSentEvent have finished. Hold only the cache while publishing,
        // so an immediate acknowledgement cannot race its pending signature.
        let overflow = {
            let mut cache = recipient
                .signature_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self.queue_outgoing(super::super::OutgoingPacket::normal(data), packet_len) {
                return;
            }
            if let Some(signature) = &message.signature {
                cache.add_seen_signature(signature);
                cache.last_seen_validator.add_pending(signature);
            }
            let mut state = self
                .chat_order
                .outgoing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.index = state.index.saturating_add(1);
            cache.last_seen_validator.tracked_messages_count() > 4096
        };
        if overflow {
            self.try_kick(&TextComponent::translate(
                "multiplayer.disconnect.too_many_pending_chats",
                [],
            ));
        }
    }
}
