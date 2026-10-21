use crossbeam::atomic::AtomicCell;

/// Keeps fall distance and its mutation identity in one atomic snapshot.
#[derive(Debug, Default)]
pub struct FallDistance(AtomicCell<(u32, u64)>);

#[derive(Clone, Copy)]
pub struct FallDistanceSnapshot((u32, u64));

impl FallDistance {
    #[must_use]
    pub const fn new(distance: f32) -> Self {
        Self(AtomicCell::new((distance.to_bits(), 0)))
    }

    #[must_use]
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load().0)
    }

    pub fn store(&self, distance: f32) {
        let _ = self.swap(distance);
    }

    pub fn swap(&self, distance: f32) -> f32 {
        let mut current = self.0.load();
        loop {
            let next = (distance.to_bits(), current.1.wrapping_add(1));
            match self.0.compare_exchange(current, next) {
                Ok(previous) => return f32::from_bits(previous.0),
                Err(updated) => current = updated,
            }
        }
    }

    pub fn take(&self) -> f32 {
        self.swap(0.0)
    }

    #[must_use]
    pub fn into_inner(self) -> f32 {
        f32::from_bits(self.0.into_inner().0)
    }

    pub(crate) fn snapshot(&self) -> FallDistanceSnapshot {
        FallDistanceSnapshot(self.0.load())
    }

    pub(crate) fn reset_if_unchanged(&self, snapshot: FallDistanceSnapshot) {
        let _ = self
            .0
            .compare_exchange(snapshot.0, (0.0f32.to_bits(), snapshot.0.1.wrapping_add(1)));
    }
}

impl From<f32> for FallDistance {
    fn from(distance: f32) -> Self {
        Self::new(distance)
    }
}
