#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures and decoded packets must be valid"
)]

mod dispenser;
mod fishing_ai;
mod items;
pub mod support;
mod weapons;
mod wear_lifecycle;

mod controls;
mod crossbow_admission;
mod crossbow_paid_rounds;
mod crossbow_rollback;
mod insertion;
mod lifecycle;
mod source_snapshots;
mod world_snapshot;
