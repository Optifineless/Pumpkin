//! Inherited Zombie spawn state; `ZombifiedPiglin` only overrides the reinforcement base roll.
use super::{Entity, Ordering, ZombifiedPiglinEntity};
use crate::entity::ai::goal::Goal;
use crate::entity::ai::goal::break_door::BreakDoorGoal;
impl ZombifiedPiglinEntity {
    pub(super) fn set_spawn_baby(&self, baby: bool) {
        // Zombie.setBaby and ZombifiedPiglin.BABY_DIMENSIONS (Java lines 50-52).
        self.mob_entity.set_baby_flag(
            &self.is_baby,
            pumpkin_data::tracked_data::zombie::BABY,
            baby,
        );
        let living = &self.mob_entity.living_entity;
        if let Some(speed) = living
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(&pumpkin_data::attributes::Attributes::MOVEMENT_SPEED.id)
        {
            crate::entity::mob::zombie::apply_baby_speed_modifier(speed, baby);
        }
        let entity = &living.entity;
        let dimensions = if baby {
            pumpkin_util::math::boundingbox::EntityDimensions::new(0.49, 0.98, 0.78)
        } else {
            Entity::type_dimensions(entity.entity_type)
        };
        entity.entity_dimension.store(dimensions);
        let pos = entity.pos.load();
        entity
            .bounding_box
            .store(pumpkin_util::math::boundingbox::BoundingBox::new_from_pos(
                pos.x,
                pos.y,
                pos.z,
                &dimensions,
            ));
    }
    pub(super) fn set_spawn_doors(&self, enabled: bool) {
        // Zombie.setCanBreakDoors also applies to zombified piglins.
        if self.can_break_doors.swap(enabled, Ordering::Relaxed) != enabled {
            let stopped = {
                let mut goals = self
                    .mob_entity
                    .goals_selector
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if enabled {
                    goals.add_goal(1, Box::new(BreakDoorGoal::default()));
                    Vec::new()
                } else {
                    goals.remove_goals::<BreakDoorGoal>()
                }
            };
            for mut goal in stopped {
                goal.stop(self);
            }
        }
        self.mob_entity
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_can_open_doors(enabled);
    }
}
