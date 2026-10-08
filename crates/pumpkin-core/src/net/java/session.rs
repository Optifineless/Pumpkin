use std::time::{Duration, Instant};

// ServerCommonPacketListenerImpl.keepConnectionAlive.
pub(super) const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MAX_TICKS_BEFORE_LOGIN: i32 = 600;
// Fork bound on PacketUtils.ensureRunningOnSameThread's queue; allow brief tick stalls.
pub(super) const MAX_INBOUND_PACKETS: usize = 4096;
pub(super) const MAX_INBOUND_BYTES: usize = pumpkin_protocol::MAX_PACKET_DATA_SIZE;

pub(super) enum KeepAliveAction {
    Wait,
    Send(i64),
    Timeout,
}

pub(super) struct KeepAliveState {
    origin: Instant,
    sent: Instant,
    pending: Option<i64>,
    closed_at: Option<Instant>,
}

impl KeepAliveState {
    pub const fn new(now: Instant) -> Self {
        Self {
            origin: now,
            sent: now,
            pending: None,
            closed_at: None,
        }
    }

    pub fn poll(&mut self, now: Instant) -> KeepAliveAction {
        // ServerCommonPacketListenerImpl.send/checkIfClosed: terminal listeners send no more challenges.
        if let Some(closed) = self.closed_at {
            return if now.duration_since(closed) >= KEEP_ALIVE_INTERVAL {
                KeepAliveAction::Timeout
            } else {
                KeepAliveAction::Wait
            };
        }
        if now.duration_since(self.sent) < KEEP_ALIVE_INTERVAL {
            return KeepAliveAction::Wait;
        }
        if self.pending.is_some() {
            return KeepAliveAction::Timeout;
        }
        let id = now.duration_since(self.origin).as_millis() as i64;
        self.sent = now;
        self.pending = Some(id);
        KeepAliveAction::Send(id)
    }

    pub fn close_listener(&mut self, now: Instant) {
        self.closed_at.get_or_insert(now);
    }

    pub fn acknowledge(&mut self, id: i64, now: Instant) -> Option<u32> {
        if self.pending != Some(id) {
            return None;
        }
        self.pending = None;
        Some(
            now.duration_since(self.sent)
                .as_millis()
                .min(u128::from(u32::MAX)) as u32,
        )
    }
}

pub(super) fn reserve_inbound_bytes(bytes: &std::sync::atomic::AtomicUsize, length: usize) -> bool {
    use std::sync::atomic::Ordering;
    bytes
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |bytes| {
            bytes
                .checked_add(length)
                .filter(|sum| *sum <= MAX_INBOUND_BYTES)
        })
        .is_ok()
}

pub(super) fn reserve_inbound(
    packets: usize,
    bytes: &std::sync::atomic::AtomicUsize,
    length: usize,
) -> bool {
    packets < MAX_INBOUND_PACKETS && reserve_inbound_bytes(bytes, length)
}

pub(super) async fn await_pending_write<T>(
    close: &tokio_util::sync::CancellationToken,
    write: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = close.cancelled() => None,
        result = tokio::time::timeout(READ_TIMEOUT, write) => result.ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_queue_survives_a_thirteen_second_tick_stall() {
        let bytes = std::sync::atomic::AtomicUsize::new(0);
        // ClientTickEnd and movement at 20 Hz must fit while the game tick stalls.
        for packets in 0..(13 * 40) {
            assert!(reserve_inbound(packets, &bytes, 32));
        }
        assert!(!reserve_inbound(MAX_INBOUND_PACKETS, &bytes, 32));
        assert!(!reserve_inbound(0, &bytes, MAX_INBOUND_BYTES));
    }

    #[test]
    fn inbound_byte_budget_rejects_growth_and_overflow() {
        let bytes = std::sync::atomic::AtomicUsize::new(0);
        assert!(reserve_inbound_bytes(&bytes, MAX_INBOUND_BYTES - 1));
        assert!(!reserve_inbound_bytes(&bytes, 2));
        assert!(!reserve_inbound_bytes(&bytes, usize::MAX));
        assert!(reserve_inbound_bytes(&bytes, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn pending_write_is_bounded_and_stop_cancels_it() {
        let close = tokio_util::sync::CancellationToken::new();
        let started = tokio::time::Instant::now();
        let timed = tokio::time::timeout(
            READ_TIMEOUT + Duration::from_secs(1),
            await_pending_write(&close, std::future::pending::<()>()),
        )
        .await;
        assert!(timed.is_ok_and(|result| result.is_none()));
        assert_eq!(started.elapsed(), READ_TIMEOUT);
        let stopped = tokio_util::sync::CancellationToken::new();
        stopped.cancel();
        let started = tokio::time::Instant::now();
        assert!(
            await_pending_write(&stopped, std::future::pending::<()>())
                .await
                .is_none()
        );
        assert!(started.elapsed().is_zero());
    }

    #[test]
    fn terminal_listener_suppresses_challenges_and_bounds_transition() {
        let now = Instant::now();
        let mut state = KeepAliveState::new(now);
        state.close_listener(now + Duration::from_secs(14));
        assert!(matches!(
            state.poll(now + Duration::from_secs(15)),
            KeepAliveAction::Wait
        ));
        assert!(matches!(
            state.poll(now + Duration::from_secs(29)),
            KeepAliveAction::Timeout
        ));
    }

    #[test]
    fn received_reply_survives_a_stalled_game_tick() {
        let now = Instant::now();
        let mut state = KeepAliveState::new(now);
        assert!(matches!(
            state.poll(now + Duration::from_secs(15)),
            KeepAliveAction::Send(15_000)
        ));
        assert!(
            state
                .acknowledge(15_000, now + Duration::from_secs(16))
                .is_some()
        );
        assert!(matches!(
            state.poll(now + Duration::from_secs(30)),
            KeepAliveAction::Send(_)
        ));
    }

    #[test]
    fn one_challenge_and_exact_timeout() {
        let now = Instant::now();
        let mut state = KeepAliveState::new(now);
        assert!(matches!(
            state.poll(now + Duration::from_secs(14)),
            KeepAliveAction::Wait
        ));
        assert!(matches!(
            state.poll(now + Duration::from_secs(15)),
            KeepAliveAction::Send(15_000)
        ));
        assert!(
            state
                .acknowledge(99, now + Duration::from_secs(16))
                .is_none()
        );
        assert!(matches!(
            state.poll(now + Duration::from_secs(29)),
            KeepAliveAction::Wait
        ));
        assert!(matches!(
            state.poll(now + Duration::from_secs(30)),
            KeepAliveAction::Timeout
        ));
        assert_eq!(
            state.acknowledge(15_000, now + Duration::from_secs(30)),
            Some(15_000)
        );
        assert!(
            state
                .acknowledge(15_000, now + Duration::from_secs(30))
                .is_none()
        );
    }
}
