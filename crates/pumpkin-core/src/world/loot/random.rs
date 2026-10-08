use super::{DynamicLootTable, LootContextParameters};
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::{legacy_rand::LegacyRand, xoroshiro128::Xoroshiro};
enum Source<'a> {
    Sequence(&'a mut Xoroshiro),
    Level(&'a mut LegacyRand),
    Legacy(LegacyRand),
}
pub(super) struct LootRandom<'a> {
    source: Source<'a>,
    remaining_work: usize,
}
const MAX_LOOT_WORK: usize = 65536;
impl LootRandom<'_> {
    pub(super) const fn seeded(seed: i64) -> Self {
        Self {
            source: Source::Legacy(LegacyRand::from_seed(seed as u64)),
            remaining_work: MAX_LOOT_WORK,
        }
    }
    pub(super) const fn charge_work(&mut self, amount: usize) -> bool {
        if let Some(remaining) = self.remaining_work.checked_sub(amount) {
            self.remaining_work = remaining;
            true
        } else {
            self.remaining_work = 0;
            false
        }
    }
    pub(super) fn next_f32(&mut self) -> f32 {
        match &mut self.source {
            Source::Sequence(rng) => rng.next_f32(),
            Source::Level(rng) => rng.next_f32(),
            Source::Legacy(rng) => rng.next_f32(),
        }
    }
    pub(super) fn next_bool(&mut self) -> bool {
        match &mut self.source {
            Source::Sequence(rng) => rng.next_bool(),
            Source::Level(rng) => rng.next_bool(),
            Source::Legacy(rng) => rng.next_bool(),
        }
    }
    pub(super) fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        match &mut self.source {
            Source::Sequence(rng) => rng.next_bounded_i32(bound),
            Source::Level(rng) => rng.next_bounded_i32(bound),
            Source::Legacy(rng) => rng.next_bounded_i32(bound),
        }
    }
}
pub(super) fn with_random(
    table: &DynamicLootTable,
    seed: i64,
    params: &LootContextParameters,
    action: &mut dyn FnMut(&mut LootRandom<'_>),
) {
    // LootContext.Builder.withOptionalRandomSeed/create uses LegacyRandomSource for nonzero seeds.
    if seed == 0
        && let Some(name) = &table.random_sequence
        && let Some(server) = params
            .world
            .as_ref()
            .and_then(|world| world.server.upgrade())
    {
        let world_seed = server.level_info.load().world_gen_settings.seed;
        let mut sequences = server
            .random_sequences
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Ok(id) = pumpkin_util::identifier::Identifier::parse(name) else {
            return;
        };
        let sequence = sequences.get_or_create(&id, world_seed);
        action(&mut LootRandom {
            source: Source::Sequence(sequence.random()),
            remaining_work: MAX_LOOT_WORK,
        });
        return;
    }
    if seed == 0
        && let Some(world) = &params.world
    {
        // LootContext.Builder.create falls back to the level's ongoing random source.
        let mut random = world
            .loot_random
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        action(&mut LootRandom {
            source: Source::Level(&mut random),
            remaining_work: MAX_LOOT_WORK,
        });
        return;
    }
    action(&mut LootRandom::seeded(if seed == 0 {
        pumpkin_util::random::get_seed() as i64
    } else {
        seed
    }));
}
