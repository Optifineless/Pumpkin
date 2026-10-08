use std::sync::Mutex;

// Fork queue bound: leave room for per-tick PlayerAuthInput during brief tick stalls.
const MAX_INBOUND_PACKETS: usize = 4096;

#[derive(Default)]
pub(super) struct InboundBudget(Mutex<(usize, usize)>);

impl InboundBudget {
    pub(super) fn reserve(&self, length: usize) -> bool {
        let mut usage = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(bytes) = usage
            .1
            .checked_add(length)
            .filter(|bytes| *bytes <= pumpkin_protocol::MAX_PACKET_DATA_SIZE)
        else {
            return false;
        };
        if usage.0 >= MAX_INBOUND_PACKETS {
            return false;
        }
        usage.0 += 1;
        usage.1 = bytes;
        true
    }

    pub(super) fn release(&self, length: usize) {
        let mut usage = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        usage.0 = usage.0.saturating_sub(1);
        usage.1 = usage.1.saturating_sub(length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_queue_bounds_packets_bytes_and_overflow() {
        let budget = InboundBudget::default();
        for _ in 0..4096 {
            assert!(budget.reserve(1));
        }
        assert!(!budget.reserve(1));
        budget.release(1);
        assert!(!budget.reserve(usize::MAX));
        assert!(!budget.reserve(pumpkin_protocol::MAX_PACKET_DATA_SIZE));
        assert!(budget.reserve(1));
    }
}
