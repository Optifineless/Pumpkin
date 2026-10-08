use std::sync::Arc;

use crate::entity::{
    Entity,
    mob::{Mob, MobEntity, equipment::RegionalDifficulty, skeleton::SkeletonEntityBase},
};
use crate::world::World;

pub struct SkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl SkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        let skeleton = Self { entity };
        Arc::new(skeleton)
    }
}

impl Mob for SkeletonEntity {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn crate::entity::EntityBase>,
        _view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
        _reason: crate::entity::mob::spawn::SpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        let world = &entity.get_entity().world.load_full();
        self.entity
            .finalize_skeleton_spawn(self, world, difficulty, group_data)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn populate_default_equipment_slots(
        &self,
        world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        self.entity
            .populate_default_equipment_slots(world, difficulty);
    }

    fn populate_default_equipment_enchantments(&self, difficulty: &RegionalDifficulty) {
        self.entity
            .populate_default_equipment_enchantments(difficulty);
    }
}
