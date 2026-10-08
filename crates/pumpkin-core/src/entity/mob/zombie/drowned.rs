use std::sync::Arc;

use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};
use pumpkin_nbt::compound::NbtCompound;

pub struct DrownedEntity {
    entity: Arc<ZombieEntityBase>,
}

impl DrownedEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        let zombie = Self { entity };
        let mob_arc = Arc::new(zombie);
        // Fix duplicated since already in ZombieEntity::new()
        {
            //let mut target_selector = mob_arc.entity.mob_entity.target_selector.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

            // TODO
            // target_selector.add_goal(
            //     2,
            //     ActiveTargetGoal::with_default(
            //         &mob_arc.entity.mob_entity,
            //         &EntityType::PLAYER,
            //         true,
            //     ),
            // );
        };

        mob_arc
    }

    #[must_use]
    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let entity = ZombieEntityBase::with_can_break_doors(entity, can_break_doors);
        let zombie = Self { entity };
        Arc::new(zombie)
    }
}

impl Mob for DrownedEntity {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn crate::entity::EntityBase>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &crate::entity::mob::equipment::RegionalDifficulty,
        reason: crate::entity::mob::spawn::SpawnReason,
        group_data: Option<crate::entity::mob::spawn::SpawnGroupData>,
    ) -> Option<crate::entity::mob::spawn::SpawnGroupData> {
        let world = &entity.get_entity().world.load_full();
        let group_data = self
            .entity
            .finalize_zombie_spawn(self, entity, view, difficulty, reason, group_data);
        // Drowned.finalizeSpawn adds a guaranteed offhand shell after Zombie's finalizer.
        let mob = self.get_mob_entity();
        let offhand_empty = mob
            .living_entity
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&pumpkin_data::data_component_impl::EquipmentSlot::OFF_HAND)
            .is_empty();
        if offhand_empty && rand::random::<f32>() < 0.03 {
            mob.set_item_slot(
                &pumpkin_data::data_component_impl::EquipmentSlot::OFF_HAND,
                pumpkin_data::item_stack::ItemStack::new(
                    1,
                    &pumpkin_data::item::Item::NAUTILUS_SHELL,
                ),
            );
            mob.set_guaranteed_drop(&pumpkin_data::data_component_impl::EquipmentSlot::OFF_HAND);
        }
        super::drowned_spawn::try_nautilus(self, entity, world, view, reason, rand::random());
        group_data
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
