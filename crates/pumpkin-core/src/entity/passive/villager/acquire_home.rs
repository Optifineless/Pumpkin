use pumpkin_util::math::position::BlockPos;
use rand::RngExt;
use rustc_hash::FxHashMap;

// AcquirePoi.create / JitteredLinearRetry: filter failed positions before limiting the batch.
const RATE: i64 = 20;
const MIN_INTERVAL_INCREASE: i64 = 40;
const MAX_RETRY_PATHFINDING_INTERVAL: i64 = 400;

#[derive(Default)]
pub(super) struct AcquireHome {
    next_start: Option<i64>,
    retries: FxHashMap<BlockPos, Retry>,
}

struct Retry {
    previous: i64,
    next: i64,
    delay: i64,
}

impl AcquireHome {
    pub(super) fn ready(&mut self, now: i64, random: &mut impl rand::Rng) -> bool {
        let Some(next) = self.next_start else {
            self.next_start = Some(now + random.random_range(0..RATE));
            return false;
        };
        if now < next {
            return false;
        }
        self.next_start = Some(now + RATE + random.random_range(0..RATE));
        self.retries
            .retain(|_, retry| now - retry.previous < MAX_RETRY_PATHFINDING_INTERVAL);
        true
    }

    pub(super) fn candidates(
        &mut self,
        homes: Vec<BlockPos>,
        now: i64,
        random: &mut impl rand::Rng,
    ) -> Vec<BlockPos> {
        homes
            .into_iter()
            .filter(|pos| {
                let Some(retry) = self.retries.get_mut(pos) else {
                    return true;
                };
                if now < retry.next {
                    return false;
                }
                retry.mark_attempt(now, random);
                true
            })
            .take(5)
            .collect()
    }

    pub(super) fn failed(&mut self, pos: BlockPos, now: i64, random: &mut impl rand::Rng) {
        self.retries.entry(pos).or_insert_with(|| {
            let mut retry = Retry {
                previous: now,
                next: now,
                delay: 0,
            };
            retry.mark_attempt(now, random);
            retry
        });
    }
    pub(super) fn clear(&mut self) {
        self.retries.clear();
    }
}

impl Retry {
    fn mark_attempt(&mut self, now: i64, random: &mut impl rand::Rng) {
        self.previous = now;
        self.delay =
            (self.delay + MIN_INTERVAL_INCREASE + random.random_range(0..MIN_INTERVAL_INCREASE))
                .min(MAX_RETRY_PATHFINDING_INTERVAL);
        self.next = now + self.delay;
    }
}
