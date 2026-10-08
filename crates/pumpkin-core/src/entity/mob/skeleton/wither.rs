use std::sync::Arc;

use crate::entity::{
    Entity,
    mob::{Mob, MobEntity, skeleton::SkeletonEntityBase},
};

pub struct WitherSkeletonEntity {
    entity: Arc<SkeletonEntityBase>,
}

impl WitherSkeletonEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = SkeletonEntityBase::new(entity);
        let skeleton = Self { entity };
        Arc::new(skeleton)
    }
}

impl Mob for WitherSkeletonEntity {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn crate::entity::EntityBase>,
        _view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
        _reason: crate::entity::mob::spawn::SpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        let world = &entity.get_entity().world.load_full();
        // WitherSkeleton.finalizeSpawn changes attack damage after AbstractSkeleton's finalizer.
        let group_data = self
            .entity
            .finalize_skeleton_spawn(self, world, difficulty, group_data);
        self.entity
            .mob_entity
            .living_entity
            .set_attribute_base(&pumpkin_data::attributes::Attributes::ATTACK_DAMAGE, 4.0);
        group_data
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }
}
