use super::{MobData, PathNavigation};
use crate::entity::living::LivingEntity;
use crate::entity::mob::MobEntity;
use pumpkin_data::attributes::Attributes;
use std::sync::atomic::Ordering;

impl PathNavigation {
    pub(super) fn path_type_at(
        &mut self,
        living: &LivingEntity,
        pos: super::BlockPos,
    ) -> super::PathType {
        use super::NodeEvaluator;
        // MoveControl.isWalkable calls getPathType directly, without NodeEvaluator.prepare/done.
        let mut context = super::PathfindingContext::without_cache(
            living.entity.block_pos.load().0,
            living.entity.world.load_full(),
        );
        match &mut self.evaluator {
            super::EvaluatorKind::Walk(e) => e.get_path_type(&mut context, pos.0),
            super::EvaluatorKind::Fly(e) => e.get_path_type(&mut context, pos.0),
            super::EvaluatorKind::Swim(e) => e.get_path_type(&mut context, pos.0),
            super::EvaluatorKind::Amphibious(e) => e.get_path_type(&mut context, pos.0),
        }
    }

    // NodeEvaluator.prepare captures the live mob, not defaults for a zombie in the overworld.
    pub(super) fn mob_data(&self, mob: &MobEntity) -> MobData {
        let living = &mob.living_entity;
        let entity = &living.entity;
        let world = entity.world.load();
        let mut data = MobData::new(
            entity.pos.load(),
            entity.width(),
            entity.height(),
            living.get_attribute_value(&Attributes::STEP_HEIGHT) as f32,
        );
        data.on_ground = entity.on_ground.load(Ordering::Relaxed);
        data.is_in_water = entity.touching_water.load(Ordering::Relaxed);
        data.can_swim = self.can_float;
        data.can_stand_on_lava = self.stands_on_lava;
        data.min_y = world.min_y;
        data.sea_level = world.sea_level;
        data.fall_distance = living.fall_distance.load();
        // Mob.getMaxFallDistance -> LivingEntity.getComfortableFallDistance.
        let has_target = mob
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if has_target && entity.entity_type == &pumpkin_data::entity::EntityType::CREEPER {
            // Creeper.getMaxFallDistance overrides Mob's difficulty-based sacrifice.
            data.max_fall_distance = (living.health.load() - 1.0 + 3.0).floor();
        } else if has_target {
            let sacrifice = (living.health.load()
                - living.get_attribute_value(&Attributes::MAX_HEALTH) as f32 * 0.33)
                as i32;
            let difficulty = world.level_info.load().difficulty as i32;
            data.max_fall_distance = ((sacrifice - (3 - difficulty) * 4).max(0) + 3) as f32;
        }
        self.apply_pathfinding_maluses(&mut data);
        data
    }

    // PathType defaults apply independently of canFloat; only setPathfindingMalus overrides them.
    pub(super) fn apply_pathfinding_maluses(&self, data: &mut MobData) {
        for (&path_type, &malus) in &self.path_type_overrides {
            data.set_pathfinding_malus(path_type, malus);
        }
    }

    pub(super) fn prepare_evaluator(&mut self, mob: &MobEntity) {
        let living = &mob.living_entity;
        let data = self.mob_data(mob);
        let context = super::PathfindingContext::new(
            living.entity.block_pos.load().0,
            living.entity.world.load_full(),
        );
        self.evaluator.set_can_float(self.can_float);
        self.evaluator.set_can_open_doors(self.can_open_doors);
        self.evaluator.set_can_pass_doors(self.can_pass_doors);
        self.evaluator
            .set_can_walk_over_fences(self.can_walk_over_fences);
        self.evaluator.prepare(context, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{Entity, mob::MobEntity};
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
    use std::sync::Arc;

    #[tokio::test]
    async fn live_mob_target_and_prepared_evaluator_survive_a_strafe_probe() {
        let directory = tempfile::tempdir().unwrap();
        let world = crate::entity::living::test_support::armor_test_world(directory.path());
        let mob = MobEntity::new(Entity::new(
            world.clone(),
            Vector3::new(4.5, 60.0, 4.5),
            &EntityType::CREEPER,
        ));
        mob.living_entity.health.store(20.0);
        *mob.target.lock().unwrap() = Some(Arc::new(LivingEntity::new(Entity::new(
            world,
            Vector3::new(8.5, 60.0, 4.5),
            &EntityType::COW,
        ))));
        let mut navigation = PathNavigation::default();
        // Creeper.getMaxFallDistance with 20 health and a target allows a 22-block fall,
        // even before the owning mob is inserted into the world's entity list.
        assert_eq!(navigation.mob_data(&mob).max_fall_distance, 22.0);
        navigation.prepare_evaluator(&mob);
        let _ = navigation.path_type_at(&mob.living_entity, BlockPos::new(4, 60, 4));
        if let super::super::EvaluatorKind::Walk(evaluator) = &navigation.evaluator {
            assert!(evaluator.base.context.is_some());
            assert_eq!(evaluator.base.mob_data.unwrap().max_fall_distance, 22.0);
        } else {
            panic!("expected the ground node evaluator");
        }
    }
}
