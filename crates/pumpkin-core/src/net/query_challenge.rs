use hmac::{Hmac, Mac, digest::KeyInit};
use sha2::Sha256;
use std::{net::SocketAddr, time::Instant};

pub(super) struct QueryChallenges {
    key: [u8; 64],
    origin: Instant,
}

impl QueryChallenges {
    pub(super) fn new() -> Self {
        Self {
            key: rand::random(),
            origin: Instant::now(),
        }
    }

    pub(super) fn issue(&self, address: SocketAddr) -> i32 {
        self.token(address, self.origin.elapsed().as_secs() / 30)
    }

    pub(super) fn accepts(&self, address: SocketAddr, token: i32) -> bool {
        let epoch = self.origin.elapsed().as_secs() / 30;
        token == self.token(address, epoch)
            || (epoch > 0 && token == self.token(address, epoch - 1))
    }

    fn token(&self, address: SocketAddr, epoch: u64) -> i32 {
        // QueryThreadGs4.Challenge binds tokens to their source and expires them.
        // Fork hardening: authenticated stateless cookies keep spoofed sources from filling a table.
        let mut mac = <Hmac<Sha256> as KeyInit>::new((&self.key).into());
        mac.update(&address.ip().to_canonical().to_string().into_bytes());
        mac.update(&address.port().to_be_bytes());
        mac.update(&epoch.to_be_bytes());
        let digest = mac.finalize().into_bytes();
        i32::from_be_bytes([digest[0] & 0x7f, digest[1], digest[2], digest[3]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_flood_cannot_evict_an_issued_challenge() {
        let challenges = QueryChallenges::new();
        let first = SocketAddr::from(([192, 0, 2, 1], 1));
        let token = challenges.issue(first);
        for port in 2..=65_535 {
            let _ = challenges.issue(SocketAddr::from(([192, 0, 2, 1], port)));
        }
        assert!(challenges.accepts(first, token));
        assert!(!challenges.accepts(SocketAddr::from(([192, 0, 2, 2], 1)), token));
        assert_ne!(challenges.token(first, 0), challenges.token(first, 2));
    }
}
