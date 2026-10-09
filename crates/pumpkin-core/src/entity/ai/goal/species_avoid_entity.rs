use std::sync::{Arc, Weak};

use pumpkin_data::entity::EntityType;
use rand::RngExt;

use super::{Controls, Goal, avoid_entity::AvoidEntityGoal};
use crate::entity::{
    EntityBase,
    mob::Mob,
    passive::{
        cat::CatEntity, llama::LlamaEntity, ocelot::OcelotEntity, tamable::TamableAnimal,
        trader_llama::TraderLlamaEntity, wolf::WolfEntity,
    },
};

pub struct CatAvoidEntityGoal {
    cat: Weak<CatEntity>,
    inner: AvoidEntityGoal,
}

impl CatAvoidEntityGoal {
    #[must_use]
    pub fn new(cat: &Arc<CatEntity>) -> Box<Self> {
        let weak = Arc::downgrade(cat);
        let predicate_cat = weak.clone();
        Box::new(Self {
            cat: weak,
            inner: AvoidEntityGoal::new(
                &EntityType::PLAYER,
                16.0,
                0.8,
                1.33,
                Some(Box::new(move |_, _| {
                    predicate_cat.upgrade().is_some_and(|cat| !cat.is_tame())
                })),
            ),
        })
    }
}

impl Goal for CatAvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // Cat.CatAvoidEntityGoal.canUse / canContinueToUse.
        self.cat.upgrade().is_some_and(|cat| !cat.is_tame()) && self.inner.can_start(mob)
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.cat.upgrade().is_some_and(|cat| !cat.is_tame()) && self.inner.should_continue(mob)
    }
    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }
    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

pub struct OcelotAvoidEntityGoal {
    ocelot: Weak<OcelotEntity>,
    inner: AvoidEntityGoal,
}

impl OcelotAvoidEntityGoal {
    #[must_use]
    pub fn new(ocelot: &Arc<OcelotEntity>) -> Box<Self> {
        let weak = Arc::downgrade(ocelot);
        let predicate_ocelot = weak.clone();
        Box::new(Self {
            ocelot: weak,
            inner: AvoidEntityGoal::new(
                &EntityType::PLAYER,
                16.0,
                0.8,
                1.33,
                Some(Box::new(move |_, _| {
                    predicate_ocelot
                        .upgrade()
                        .is_some_and(|ocelot| !ocelot.is_trusting())
                })),
            ),
        })
    }
}

impl Goal for OcelotAvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // Ocelot.OcelotAvoidEntityGoal.canUse / canContinueToUse.
        self.ocelot
            .upgrade()
            .is_some_and(|ocelot| !ocelot.is_trusting())
            && self.inner.can_start(mob)
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.ocelot
            .upgrade()
            .is_some_and(|ocelot| !ocelot.is_trusting())
            && self.inner.should_continue(mob)
    }
    fn start(&mut self, mob: &dyn Mob) {
        self.inner.start(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        self.inner.tick(mob);
    }
    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

pub struct WolfAvoidEntityGoal {
    wolf: Weak<WolfEntity>,
    inner: AvoidEntityGoal,
}

impl WolfAvoidEntityGoal {
    #[must_use]
    pub fn new(wolf: &Arc<WolfEntity>) -> Box<Self> {
        Box::new(Self {
            wolf: Arc::downgrade(wolf),
            inner: AvoidEntityGoal::new(&EntityType::LLAMA, 24.0, 1.5, 1.5, None),
        })
    }

    fn can_start_with_roll(&mut self, mob: &dyn Mob, roll: impl FnOnce() -> i32) -> bool {
        // Wolf.WolfAvoidEntityGoal.canUse rolls only after super.canUse admits the nearest llama's path.
        self.inner.can_start(mob)
            && self.wolf.upgrade().is_some_and(|wolf| !wolf.is_tame())
            && self
                .inner
                .threat()
                .and_then(llama_strength)
                .is_some_and(|strength| strength >= roll())
    }
}

fn llama_strength(target: &dyn EntityBase) -> Option<i32> {
    // Wolf.WolfAvoidEntityGoal.avoidLlama calls Llama.getStrength, including TraderLlama subclasses.
    target
        .cast_any()
        .downcast_ref::<LlamaEntity>()
        .map(LlamaEntity::get_strength)
        .or_else(|| {
            target
                .cast_any()
                .downcast_ref::<TraderLlamaEntity>()
                .map(TraderLlamaEntity::get_strength)
        })
}

impl Goal for WolfAvoidEntityGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        // Wolf.WolfAvoidEntityGoal.avoidLlama: random.nextInt(5).
        self.can_start_with_roll(mob, || mob.get_random().random_range(0..5))
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.inner.should_continue(mob)
    }
    fn start(&mut self, mob: &dyn Mob) {
        // Wolf.WolfAvoidEntityGoal.start / tick clear the attack target while fleeing.
        mob.get_mob_entity().set_target(None);
        self.inner.start(mob);
    }
    fn tick(&mut self, mob: &dyn Mob) {
        mob.get_mob_entity().set_target(None);
        self.inner.tick(mob);
    }
    fn stop(&mut self, mob: &dyn Mob) {
        self.inner.stop(mob);
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}

#[cfg(test)]
pub(super) mod tests;
