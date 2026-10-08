use crate::entity::EntityBase;
use crate::entity::ai::{control::drowned_move_control::DrownedMoveControl, pathfinder::Navigator};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::entity::mob::zombie::ZombieEntityBase;
use crate::entity::{
    Entity,
    mob::{Mob, MobEntity},
};
use pumpkin_nbt::compound::NbtCompound;

pub struct DrownedEntity {
    entity: Arc<ZombieEntityBase>,
    searching_for_land: AtomicBool,
}

impl DrownedEntity {
    /// `DrownedSwimUpGoal` owns this flag from start until stop.
    /// Stub input for the unregistered `DrownedGoToBeachGoal` and `DrownedSwimUpGoal`.
    pub fn set_searching_for_land(&self, value: bool) {
        self.searching_for_land.store(value, Ordering::Relaxed);
    }
    pub fn is_searching_for_land(&self) -> bool {
        self.searching_for_land.load(Ordering::Relaxed)
    }
    /// Drowned.wantsToSwim: land search or a target in water.
    pub fn wants_to_swim(&self) -> bool {
        self.is_searching_for_land()
            || self
                .entity
                .mob_entity
                .get_target()
                .is_some_and(|t| t.get_entity().touching_water.load(Ordering::Relaxed))
    }

    pub fn new(entity: Entity) -> Arc<Self> {
        let entity = ZombieEntityBase::new(entity);
        // Drowned constructor / createNavigation.
        let navigation = Navigator::amphibious(false);
        entity
            .mob_entity
            .configure_movement(navigation, DrownedMoveControl::default());
        let zombie = Self {
            entity,
            searching_for_land: AtomicBool::new(false),
        };
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
        // Drowned constructor / createNavigation.
        let navigation = Navigator::amphibious(false);
        entity
            .mob_entity
            .configure_movement(navigation, DrownedMoveControl::default());
        let zombie = Self {
            entity,
            searching_for_land: AtomicBool::new(false),
        };
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

    fn mob_is_pushed_by_fluids(&self) -> bool {
        !self.get_entity().is_swimming()
    }
    // Drowned.updateSwimming / travelInWater.
    fn mob_tick(&self, _caller: &dyn EntityBase) {
        self.get_entity().set_swimming(
            !self.entity.mob_entity.is_no_ai()
                && self.get_entity().is_submerged_in_water()
                && self.wants_to_swim(),
        );
    }
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        self.get_entity().is_submerged_in_water()
            && self.wants_to_swim()
            && crate::entity::mob::movement::travel_in_water(self, caller, 0.01, false)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.entity.mob_entity
    }

    fn get_base_experience_reward(&self) -> u32 {
        Mob::get_base_experience_reward(self.entity.as_ref())
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
