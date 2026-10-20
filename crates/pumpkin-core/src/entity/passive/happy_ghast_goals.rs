use super::HappyGhastEntity;
use crate::entity::{
    EntityBase,
    ai::{
        goal::{Controls, Goal, swim::SwimGoal, to_goal_ticks},
        target_predicate::TargetPredicate,
    },
    mob::Mob,
    player::Player,
};
use pumpkin_data::{
    attributes::Attributes,
    tag::{self, Taggable},
};
use std::sync::{Arc, PoisonError};

fn happy(mob: &dyn Mob) -> Option<&HappyGhastEntity> {
    mob.cast_any().downcast_ref()
}

#[derive(Default)]
pub(super) struct HappyFloatGoal(SwimGoal);
impl Goal for HappyFloatGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        happy(mob).is_some_and(|h| !h.is_on_still_timeout()) && self.0.can_start(mob)
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        happy(mob).is_some_and(|h| !h.is_on_still_timeout()) && self.0.should_continue(mob)
    }
    fn tick(&mut self, mob: &dyn Mob) {
        if happy(mob).is_some_and(|h| !h.is_on_still_timeout()) {
            self.0.tick(mob);
        }
    }
    fn should_run_every_tick(&self) -> bool {
        true
    }
    fn controls(&self) -> Controls {
        Controls::JUMP
    }
}

#[derive(Default)]
pub(super) struct TemptGoal {
    player: Option<Arc<Player>>,
    cooldown: i32,
}
impl TemptGoal {
    fn eligible(ghast: &HappyGhastEntity, player: &Arc<Player>) -> bool {
        let entity = ghast.get_entity();
        let world = entity.world.load();
        if entity.is_removed() || player.is_spectator() || !world.can_respawn_player(player) {
            return false;
        }
        let range = ghast
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::TEMPT_RANGE);
        if !TargetPredicate::create_non_attackable()
            .ignore_visibility()
            .set_base_max_distance(range)
            .test(&world, Some(ghast), player.as_ref())
        {
            return false;
        }
        let tag = if ghast.wearing_body() {
            &tag::Item::MINECRAFT_HAPPY_GHAST_FOOD
        } else {
            &tag::Item::MINECRAFT_HAPPY_GHAST_TEMPT_ITEMS
        };
        [
            player.inventory().held_item(),
            player.inventory().off_hand_item(),
        ]
        .iter()
        .any(|stack| !stack.is_empty() && stack.item.has_tag(tag))
    }
    fn wait(mob: &dyn Mob) {
        mob.get_mob_entity()
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_wait();
    }
}
impl Goal for TemptGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return false;
        }
        let Some(ghast) = happy(mob) else {
            return false;
        };
        let range = ghast
            .mob_entity
            .living_entity
            .get_attribute_value(&Attributes::TEMPT_RANGE);
        let world = mob.get_entity().world.load();
        self.player = world.get_nearest_player(mob.get_entity().pos.load(), range, |player| {
            Self::eligible(ghast, player)
        });
        self.player.is_some()
    }
    fn should_continue(&mut self, mob: &dyn Mob) -> bool {
        self.can_start(mob)
    }
    fn tick(&mut self, mob: &dyn Mob) {
        let Some(ghast) = happy(mob) else {
            return;
        };
        let Some(player) = &self.player else {
            return;
        };
        // Selectors need not reevaluate on this tick; validate live hands/session before steering.
        if !Self::eligible(ghast, player) {
            Self::wait(mob);
            return;
        }
        let pos = player.get_entity().pos.load();
        let eye = player.get_entity().get_eye_pos();
        let mob_pos = mob.get_entity().pos.load();
        let look_yaw = mob.get_max_head_rotation() + 20.0;
        let look_pitch = mob.get_max_look_pitch_change();
        ghast
            .mob_entity
            .look_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .look_at_with_range(eye.x, eye.y, eye.z, look_yaw, look_pitch);
        if mob_pos.squared_distance_to_vec(&pos) < 7.0 * 7.0 {
            Self::wait(mob);
        } else {
            let target = mob_pos + (eye - mob_pos) * ghast.random_double();
            ghast
                .mob_entity
                .move_control
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_wanted_position(target.x, target.y, target.z, 1.0);
        }
    }
    fn stop(&mut self, mob: &dyn Mob) {
        self.player = None;
        Self::wait(mob);
        self.cooldown = to_goal_ticks(100);
    }
    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

pub(super) struct RandomFloatAroundGoal;
impl Goal for RandomFloatAroundGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let control = mob
            .get_mob_entity()
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !control.has_wanted() {
            return true;
        }
        let distance = (control.wanted_position().0 - mob.get_entity().pos.load()).length_squared();
        !distance.is_nan() && !(1.0..=3600.0).contains(&distance)
    }
    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        false
    }
    fn start(&mut self, mob: &dyn Mob) {
        let Some(ghast) = happy(mob) else {
            return;
        };
        let target = ghast.suitable_fly_to_position();
        ghast
            .mob_entity
            .move_control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_wanted_position(target.x, target.y, target.z, 1.0);
    }
    fn controls(&self) -> Controls {
        Controls::MOVE
    }
}
