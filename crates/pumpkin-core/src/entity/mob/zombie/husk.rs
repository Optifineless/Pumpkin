use std::sync::Arc;

use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};
use pumpkin_nbt::compound::NbtCompound;

pub struct HuskEntity {
    entity: Arc<ZombieEntityBase>,
}

impl HuskEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        let zombie = Self { entity };
        Arc::new(zombie)
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity = ZombieEntityBase::with_can_break_doors(entity, can_break_doors);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

impl Mob for HuskEntity {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn crate::entity::EntityBase>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
        reason: crate::entity::mob::spawn::SpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        self.entity
            .finalize_zombie_spawn(self, entity, view, difficulty, reason, group_data)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn get_base_experience_reward(&self) -> u32 {
        self.entity.get_base_experience_reward()
    }

    fn spawn_as_baby(&self) -> bool {
        self.entity.spawn_as_baby()
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        self.entity.mob_write_nbt(nbt);
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.entity.mob_read_nbt(nbt);
    }
}
