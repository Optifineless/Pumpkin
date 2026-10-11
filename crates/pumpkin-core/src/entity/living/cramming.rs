// LivingEntity.pushEntities (3224): one in four crowded ticks admits cramming damage.
pub(super) fn damage_roll() -> bool {
    #[cfg(test)]
    if let Some(roll) = TEST_ROLL.with(std::cell::Cell::get) {
        return roll;
    }
    rand::random::<u32>().is_multiple_of(4)
}

#[cfg(test)]
thread_local! {
    static TEST_ROLL: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_damage_roll(action: impl FnOnce()) {
    struct Reset(Option<bool>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ROLL.with(|roll| roll.set(self.0));
        }
    }
    let _reset = Reset(TEST_ROLL.with(|roll| roll.replace(Some(true))));
    action();
}
