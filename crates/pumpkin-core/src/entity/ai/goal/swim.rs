use std::sync::atomic::Ordering;

use super::{Controls, Goal};
use crate::entity::mob::Mob;
use rand::RngExt;

pub struct SwimGoal {
    goal_control: Controls,
    gate: Option<super::revenge::MobFilter>,
}

impl Default for SwimGoal {
    fn default() -> Self {
        Self {
            goal_control: Controls::JUMP,
            gate: None,
        }
    }
}

impl SwimGoal {
    /// Applies a species condition when starting, continuing and requesting jumps.
    #[must_use]
    pub const fn gated_by(mut self, gate: super::revenge::MobFilter) -> Self {
        self.gate = Some(gate);
        self
    }

    fn is_in_fluid(mob: &dyn Mob) -> bool {
        let living = &mob.get_mob_entity().living_entity;
        let entity = &living.entity;
        let in_water = entity.touching_water.load(Ordering::SeqCst)
            && entity.water_height.load() > living.get_swim_height();
        in_water || entity.touching_lava.load(Ordering::SeqCst)
    }
}

impl Goal for SwimGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.gate.is_some_and(|gate| !gate(mob)) {
            return false;
        }
        mob.get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_float(true);
        Self::is_in_fluid(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.gate.is_none_or(|gate| gate(mob)) && Self::is_in_fluid(mob)
    }

    fn tick(&mut self, mob: &dyn Mob) {
        // CreakingAi's Brain Swim remains conditional on canMove between selector updates.
        if self.gate.is_some_and(|gate| !gate(mob)) {
            return;
        }
        // Vanilla FloatGoal.tick requests a jump for this AI tick only.
        if mob.get_random().random::<f32>() < 0.8 {
            mob.get_mob_entity()
                .jump_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .jump();
        }
    }

    fn should_run_every_tick(&self) -> bool {
        true
    }

    fn controls(&self) -> Controls {
        self.goal_control
    }
}
