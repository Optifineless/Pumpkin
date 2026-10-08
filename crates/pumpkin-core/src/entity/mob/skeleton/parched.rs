use std::sync::Arc;

use crate::entity::{
    Entity,
    mob::{Mob, MobEntity, skeleton::SkeletonEntityBase},
};

pub struct ParchedSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl ParchedSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        let parched = Self { entity };
        Arc::new(parched)
    }
}

impl Mob for ParchedSkeletonEntity {
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
}
