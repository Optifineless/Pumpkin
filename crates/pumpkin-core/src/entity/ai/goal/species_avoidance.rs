//! Species overrides of vanilla's `AvoidEntityGoal`.
use pumpkin_data::entity::EntityType;
use rand::RngExt;

use super::{Controls, Goal, avoid_entity::AvoidEntityGoal};
use crate::entity::{
    EntityBase,
    mob::Mob,
    passive::{llama::LlamaEntity, ocelot::OcelotEntity, trader_llama::TraderLlamaEntity},
};

pub struct CatAvoidEntityGoal {
    inner: AvoidEntityGoal,
}

impl CatAvoidEntityGoal {
    pub(crate) fn new() -> Self {
        Self {
            inner: AvoidEntityGoal::new(&EntityType::PLAYER, 16.0, 0.8, 1.33),
        }
    }
}

impl Goal for CatAvoidEntityGoal {
    // Cat.CatAvoidEntityGoal.canUse/canContinueToUse.
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_tamed() && self.inner.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !mob.is_tamed() && self.inner.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}

pub struct OcelotAvoidEntityGoal {
    inner: AvoidEntityGoal,
}

impl OcelotAvoidEntityGoal {
    pub(crate) fn new() -> Self {
        Self {
            inner: AvoidEntityGoal::new(&EntityType::PLAYER, 16.0, 0.8, 1.33),
        }
    }

    fn is_trusting(mob: &dyn Mob) -> bool {
        mob.cast_any()
            .downcast_ref::<OcelotEntity>()
            .is_some_and(OcelotEntity::is_trusting)
    }
}

impl Goal for OcelotAvoidEntityGoal {
    // Ocelot.OcelotAvoidEntityGoal.canUse/canContinueToUse.
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        !Self::is_trusting(mob) && self.inner.can_start(mob)
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        !Self::is_trusting(mob) && self.inner.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}

pub struct WolfAvoidEntityGoal {
    inner: AvoidEntityGoal,
}

impl WolfAvoidEntityGoal {
    pub(crate) fn new() -> Self {
        Self {
            inner: AvoidEntityGoal::new(&EntityType::LLAMA, 24.0, 1.5, 1.5),
        }
    }

    // Wolf.WolfAvoidEntityGoal.avoidLlama rolls only for the nearest admitted llama.
    fn avoid_llama(mob: &dyn Mob, target: &dyn EntityBase) -> bool {
        let strength = target.cast_any().downcast_ref::<LlamaEntity>().map_or_else(
            || {
                target
                    .cast_any()
                    .downcast_ref::<TraderLlamaEntity>()
                    .map(TraderLlamaEntity::get_strength)
            },
            |llama| Some(llama.get_strength()),
        );
        strength.is_some_and(|strength| strength >= mob.get_random().random_range(0..5))
    }
}

impl Goal for WolfAvoidEntityGoal {
    // Wolf.WolfAvoidEntityGoal.canUse deliberately checks strength after base selection/path.
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        self.inner.can_start(mob)
            && !mob.is_tamed()
            && self
                .inner
                .target()
                .is_some_and(|target| Self::avoid_llama(mob, target.as_ref()))
    }

    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.inner.should_continue(mob)
    }

    fn start(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_target(None);
        self.inner.start(mob);
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }

    fn tick(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_target(None);
        self.inner.tick(mob);
    }

    fn controls(&self) -> Controls {
        self.inner.controls()
    }
}
