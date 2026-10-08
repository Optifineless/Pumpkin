use crate::entity::ai::{
    control::guardian_move_control::GuardianMoveControl, pathfinder::Navigator,
};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::potion::Effect;
use pumpkin_protocol::java::client::play::{CGameEvent, GameEvent};

use crate::entity::{
    Entity, EntityBase,
    ai::goal::{
        active_target::ActiveTargetGoal, look_around::RandomLookAroundGoal,
        look_at_entity::LookAtEntityGoal, wander_around::WanderAroundGoal,
    },
    mob::{Mob, MobEntity},
};

pub struct ElderGuardianEntity {
    pub mob_entity: MobEntity,
    tick_count: AtomicI32,
}

impl ElderGuardianEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Guardian constructor / createNavigation, inherited by ElderGuardian.
        let navigation = Navigator::water_bound(false);
        mob_entity.configure_movement(navigation, GuardianMoveControl::default());
        let guardian = Self {
            mob_entity,
            tick_count: AtomicI32::new(0),
        };
        let mob_arc = Arc::new(guardian);
        let mob_weak: Weak<dyn Mob> = {
            let mob_arc: Arc<dyn Mob> = mob_arc.clone();
            Arc::downgrade(&mob_arc)
        };

        {
            let mut goal_selector = mob_arc
                .mob_entity
                .goals_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            goal_selector.add_goal(4, Box::new(WanderAroundGoal::swimming(1.0, 80)));
            goal_selector.add_goal(
                5,
                LookAtEntityGoal::with_default(mob_weak.clone(), &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(6, Box::new(RandomLookAroundGoal::default()));

            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            target_selector.add_goal(
                1,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
        };

        mob_arc
    }
}

impl Mob for ElderGuardianEntity {
    // Guardian.getMaxHeadXRot.
    fn get_max_look_pitch_change(&self) -> f32 {
        180.0
    }

    // Guardian.travelInWater.
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        let sink = !self
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_moving()
            && self.mob_entity.get_target().is_none();
        crate::entity::mob::movement::travel_in_water(self, caller, 0.1, sink)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn mob_tick(&self, _caller: &dyn EntityBase) {
        super::movement::flop_on_land(self, 0.4);
        let entity = &self.mob_entity.living_entity.entity;
        if !entity.is_alive() {
            return;
        }

        let ticks = self.tick_count.fetch_add(1, Ordering::Relaxed) + 1;
        if ticks % 1200 == 0 {
            let pos = entity.pos.load();
            let world = entity.world.load();
            let players = world.get_nearby_players(pos, 50.0);
            let packet = CGameEvent::new(GameEvent::PlayElderGuardianMobAppearance, 1.0);

            for player in players {
                player.try_send_client_packet(&packet);
                player.living_entity.add_effect(Effect {
                    effect_type: &StatusEffect::MINING_FATIGUE,
                    duration: 6000,
                    amplifier: 2,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    blend: false,
                });
            }
        }
    }

    fn on_damage(
        &self,
        _damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        _cause: Option<&dyn EntityBase>,
    ) {
        // Guardian.hurtServer checks the direct entity for thorns, never the shooter.
        if let Some(src) = source
            && let Some(living) = src.get_living_entity()
        {
            let _ = living.damage(src, 2.0, DamageType::THORNS);
        }
    }
}
