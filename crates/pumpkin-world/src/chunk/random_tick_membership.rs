use std::sync::atomic::{AtomicBool, Ordering};

/// Per-section random tick membership, sized from the dimension rather than a machine word.
pub struct RandomTickMembership(Box<[AtomicBool]>);

impl RandomTickMembership {
    pub fn new(sections: Vec<bool>) -> Self {
        Self(sections.into_iter().map(AtomicBool::new).collect())
    }

    pub fn any(&self) -> bool {
        self.0.iter().any(|value| value.load(Ordering::Relaxed))
    }

    pub fn contains(&self, index: usize) -> bool {
        self.0[index].load(Ordering::Relaxed)
    }

    pub fn set(&self, index: usize, value: bool) {
        self.0[index].store(value, Ordering::Relaxed);
    }
}
