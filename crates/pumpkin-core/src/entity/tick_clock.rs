//! Maps `Entity.tickCount` independently of an ageable mob's growth and cooldown.

use super::{Entity, EntityBase};
use std::sync::atomic::Ordering::Relaxed;

pub(super) fn elapsed_ticks(entity: &Entity, caller: &dyn EntityBase) -> i32 {
    caller.get_mob().map_or_else(
        || entity.age.load(Relaxed),
        |mob| mob.get_mob_entity().ticks_lived.load(Relaxed),
    )
}
