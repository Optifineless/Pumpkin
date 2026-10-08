mod storage;
#[cfg(test)]
mod tests;
use std::collections::HashMap;

use pumpkin_util::identifier::Identifier;
use pumpkin_util::random::RandomImpl;
use pumpkin_util::random::xoroshiro128::Xoroshiro;

/// A single random sequence wrapper.
pub struct RandomSequence {
    rng: Xoroshiro,
}

impl RandomSequence {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            rng: Xoroshiro::from_seed(seed),
        }
    }

    /// Borrow the persistent source so loot rolls and placement advance the same sequence.
    pub const fn random(&mut self) -> &mut Xoroshiro {
        &mut self.rng
    }

    pub fn random_between_inclusive(&mut self, min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        self.rng.next_inbetween_i32(min, max)
    }
}

/// Persistent/runtime manager for server random sequences.
pub struct RandomSequences {
    salt: i32,
    include_world_seed: bool,
    include_sequence_id: bool,
    sequences: HashMap<String, RandomSequence>,
    dirty: bool,
}

impl Default for RandomSequences {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomSequences {
    #[must_use]
    pub fn new() -> Self {
        Self {
            salt: 0,
            include_world_seed: true,
            include_sequence_id: true,
            sequences: HashMap::new(),
            dirty: false,
        }
    }

    fn create_sequence(
        sequence: &Identifier,
        world_seed: i64,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) -> RandomSequence {
        // RandomSequences.createSequence XORs salt before the optional identifier hash.
        let seed = (if include_world_seed { world_seed } else { 0 } ^ i64::from(salt)) as u64;
        let key = sequence.to_string();
        RandomSequence {
            rng: Xoroshiro::from_seed_and_key(seed, include_sequence_id.then_some(key.as_str())),
        }
    }

    pub fn get_or_create(&mut self, sequence: &Identifier, world_seed: i64) -> &mut RandomSequence {
        // DirtyMarkingRandomSource: borrowing a mutable stream may advance its state.
        self.dirty = true;
        let key = sequence.to_string();
        let salt = self.salt;
        let include_world_seed = self.include_world_seed;
        let include_sequence_id = self.include_sequence_id;
        self.sequences.entry(key).or_insert_with(|| {
            Self::create_sequence(
                sequence,
                world_seed,
                salt,
                include_world_seed,
                include_sequence_id,
            )
        })
    }

    pub fn reset(&mut self, sequence: &Identifier, world_seed: i64) {
        let key = sequence.to_string();
        let sequence = Self::create_sequence(
            sequence,
            world_seed,
            self.salt,
            self.include_world_seed,
            self.include_sequence_id,
        );
        self.sequences.insert(key, sequence);
        self.dirty = true;
    }

    pub fn reset_with_options(
        &mut self,
        sequence: &Identifier,
        world_seed: i64,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) {
        let key = sequence.to_string();
        let sequence = Self::create_sequence(
            sequence,
            world_seed,
            salt,
            include_world_seed,
            include_sequence_id,
        );
        self.sequences.insert(key, sequence);
        self.dirty = true;
    }

    pub fn clear(&mut self) -> usize {
        let count = self.sequences.len();
        self.sequences.clear();
        self.dirty = true;
        count
    }

    pub const fn set_seed_defaults(
        &mut self,
        salt: i32,
        include_world_seed: bool,
        include_sequence_id: bool,
    ) {
        self.dirty = true;
        self.salt = salt;
        self.include_world_seed = include_world_seed;
        self.include_sequence_id = include_sequence_id;
    }

    #[must_use]
    pub fn get_sequence_keys(&self) -> Vec<String> {
        self.sequences.keys().cloned().collect()
    }
}
