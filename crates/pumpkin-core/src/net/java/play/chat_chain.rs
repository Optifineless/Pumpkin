use super::ChatError;
use crate::{
    entity::player::{ChatSession, Player},
    net::{
        chat::{FilterMask, PlayerChatMessage, SignedMessageBody, SignedMessageLink},
        java::JavaClient,
    },
};
use rsa::{
    RsaPublicKey,
    pkcs1v15::{Signature, VerifyingKey},
    pkcs8::DecodePublicKey,
    signature::Verifier,
};
use sha2::Sha256;
use uuid::Uuid;

#[derive(Default)]
pub struct SignedMessageChain {
    key: Option<VerifyingKey<Sha256>>,
    session: Uuid,
    index: Option<i32>,
    last_timestamp: i64,
}

impl SignedMessageChain {
    pub const fn set_chain_broken(&mut self) {
        self.index = None;
    }

    pub fn reset(&mut self, session: &ChatSession) -> Result<(), ChatError> {
        let key = RsaPublicKey::from_public_key_der(&session.public_key)
            .map_err(|_| ChatError::InvalidPublicKey)?;
        self.key = Some(VerifyingKey::new(key));
        self.session = session.session_id;
        self.index = Some(0);
        self.last_timestamp = 0;
        Ok(())
    }

    // SignedMessageChain.decoder + PlayerChatMessage.updateSignature, SignedMessageBody.updateSignature.
    pub fn unpack(
        &mut self,
        sender: Uuid,
        session: &ChatSession,
        body: SignedMessageBody,
        signature: Option<&[u8]>,
    ) -> Result<PlayerChatMessage, &'static str> {
        let signature = signature.ok_or("chat.disabled.missingProfileKey")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        if session.expires_at < now {
            return Err("chat.disabled.expiredProfileKey");
        }
        let index = self.index.ok_or("chat.disabled.chain_broken")?;
        if body.time_stamp < self.last_timestamp {
            self.index = None;
            return Err("chat.disabled.out_of_order_chat");
        }
        self.last_timestamp = body.time_stamp;
        let key = self.key.as_ref().ok_or("chat.disabled.missingProfileKey")?;
        let bytes = signature_bytes(sender, self.session, index, &body);
        let valid = Signature::try_from(signature)
            .is_ok_and(|signature| key.verify(&bytes, &signature).is_ok());
        if !valid {
            self.index = None;
            return Err("chat.disabled.invalid_signature");
        }
        self.index = index.checked_add(1);
        Ok(PlayerChatMessage::new(
            SignedMessageLink::new(index, sender, self.session),
            Some(signature.into()),
            body,
            None,
            FilterMask::PassThrough,
        ))
    }
}

fn signature_bytes(sender: Uuid, session: Uuid, index: i32, body: &SignedMessageBody) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1i32.to_be_bytes());
    bytes.extend_from_slice(sender.as_bytes());
    bytes.extend_from_slice(session.as_bytes());
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes.extend_from_slice(&body.salt.to_be_bytes());
    bytes.extend_from_slice(&body.time_stamp.div_euclid(1000).to_be_bytes());
    bytes.extend_from_slice(&(body.content.len() as i32).to_be_bytes());
    bytes.extend_from_slice(body.content.as_bytes());
    bytes.extend_from_slice(&(body.last_seen.len() as i32).to_be_bytes());
    for signature in &body.last_seen {
        bytes.extend_from_slice(signature);
    }
    bytes
}

impl JavaClient {
    pub(crate) fn apply_last_seen(
        player: &Player,
        offset: i32,
        acknowledged: &[u8],
        checksum: u8,
    ) -> Result<Vec<Box<[u8]>>, ChatError> {
        // LastSeenMessagesValidator.applyUpdate rejects bits outside the twenty-message window.
        if acknowledged.len() != 3 || acknowledged[2] & 0xf0 != 0 {
            return Err(ChatError::ChatValidationFailed);
        }
        let offset = usize::try_from(offset).map_err(|_| ChatError::ChatValidationFailed)?;
        let mut cache = player
            .signature_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let seen = cache
            .last_seen_validator
            .apply_update(offset, acknowledged)
            .map_err(|_| ChatError::ChatValidationFailed)?;
        if checksum != 0 && checksum != pumpkin_util::math::polynomial_rolling_hash(&seen) {
            return Err(ChatError::ChatValidationFailed);
        }
        if cache.last_seen_validator.tracked_messages_count() > 4096 {
            return Err(ChatError::TooManyPendingChats);
        }
        Ok(seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_replay_and_breaks_the_signed_chain() {
        // Independent OpenSSL PKCS#1/SHA-256 fixture for vanilla's root link and "hello".
        let session = ChatSession::new(
            Uuid::nil(),
            i64::MAX,
            include_bytes!("fixtures/chat-key.der").as_slice().into(),
            Box::new([]),
        );
        let signature = include_bytes!("fixtures/chat-signature.bin");
        let body = || SignedMessageBody::new("hello".into(), 1000, 0, vec![]);
        let mut chain = SignedMessageChain::default();
        chain.reset(&session).unwrap();
        assert!(
            chain
                .unpack(Uuid::nil(), &session, body(), Some(signature))
                .is_ok()
        );
        assert_eq!(
            chain
                .unpack(Uuid::nil(), &session, body(), Some(signature))
                .err(),
            Some("chat.disabled.invalid_signature")
        );
        assert_eq!(
            chain
                .unpack(Uuid::nil(), &session, body(), Some(signature))
                .err(),
            Some("chat.disabled.chain_broken")
        );
    }

    #[test]
    fn signed_body_uses_epoch_seconds() {
        let body = SignedMessageBody::new("A".into(), 1999, 2, vec![]);
        let encoded = signature_bytes(Uuid::nil(), Uuid::nil(), 3, &body);
        // Java signature fixture: version, UUIDs, index, salt, seconds, UTF-8 length/content, last-seen count.
        let mut fixture = vec![0; 40];
        fixture[3] = 1;
        fixture[39] = 3;
        fixture.extend_from_slice(&[
            0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, b'A', 0, 0, 0, 0,
        ]);
        assert_eq!(encoded, fixture);
    }
}
