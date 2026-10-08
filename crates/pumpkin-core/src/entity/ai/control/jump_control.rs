use crate::entity::ai::control::Control;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct JumpControl {
    jump: bool,
}

impl Control for JumpControl {}

impl JumpControl {
    #[must_use]
    pub const fn has_request(&self) -> bool {
        self.jump
    }
    pub fn take_request(&mut self) -> bool {
        std::mem::take(&mut self.jump)
    }

    pub const fn jump(&mut self) {
        self.jump = true;
    }

    // Vanilla JumpControl.tick consumes the accumulated requests once per AI tick.
    pub fn tick(&mut self, jumping: &AtomicBool) {
        jumping.store(self.jump, Ordering::SeqCst);
        self.jump = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_requests_last_one_tick() {
        let mut control = JumpControl::default();
        let jumping = AtomicBool::new(false);

        // A swim goal and navigation can both request a jump in the same tick.
        control.jump();
        control.jump();
        assert!(!jumping.load(Ordering::SeqCst));
        control.tick(&jumping);
        assert!(jumping.load(Ordering::SeqCst));

        control.tick(&jumping);
        assert!(!jumping.load(Ordering::SeqCst));

        control.jump();
        control.tick(&jumping);
        assert!(jumping.load(Ordering::SeqCst));
        control.tick(&jumping);
        assert!(!jumping.load(Ordering::SeqCst));
    }
}
