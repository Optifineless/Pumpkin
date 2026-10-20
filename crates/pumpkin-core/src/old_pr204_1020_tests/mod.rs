#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and decoded packets must be valid"
)]

mod dispenser;
mod fishing_ai;
mod items;
pub mod support;
mod weapons;

mod controls;
