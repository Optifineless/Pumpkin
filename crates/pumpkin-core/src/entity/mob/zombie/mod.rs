mod drowned_spawn;
pub(super) mod finalize;
use super::{Mob, MobEntity};
use crate::entity::ai::goal::break_door::BreakDoorGoal;
use crate::entity::ai::goal::destroy_egg::DestroyEggGoal;
use crate::entity::ai::goal::look_around::RandomLookAroundGoal;
use crate::entity::ai::goal::revenge::RevengeGoal;
use crate::entity::ai::goal::wander_around::WanderAroundGoal;
use crate::entity::ai::goal::zombie_attack::ZombieAttackGoal;
use crate::entity::mob::equipment::RegionalDifficulty;
use crate::entity::{
    Entity,
    ai::goal::{Goal, active_target::ActiveTargetGoal, look_at_entity::LookAtEntityGoal},
};
use crate::world::World;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::Difficulty;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

pub mod drowned;
pub mod husk;
#[allow(clippy::module_inception)]
pub mod zombie;
pub mod zombie_villager;

pub struct ZombieEntityBase {
    pub mob_entity: MobEntity,
    pub can_break_doors: AtomicBool,
    pub is_baby: AtomicBool,
}

impl ZombieEntityBase {
    pub fn new(entity: Entity) -> Arc<Self> {
        Self::with_can_break_doors(entity, false)
    }

    pub fn with_can_break_doors(entity: Entity, can_break_doors: bool) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        let zombie = Self {
            mob_entity,
            can_break_doors: AtomicBool::new(can_break_doors),
            is_baby: AtomicBool::new(false),
        };
        let mob_arc = Arc::new(zombie);
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
            let mut target_selector = mob_arc
                .mob_entity
                .target_selector
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            if can_break_doors {
                goal_selector.add_goal(1, Box::new(BreakDoorGoal::default()));
            }
            goal_selector.add_goal(2, ZombieAttackGoal::new(1.0, false));
            goal_selector.add_goal(4, DestroyEggGoal::new(1.0, 3));
            goal_selector.add_goal(7, Box::new(WanderAroundGoal::new(1.0)));
            goal_selector.add_goal(
                8,
                LookAtEntityGoal::with_default(mob_weak, &EntityType::PLAYER, 8.0),
            );
            goal_selector.add_goal(8, Box::new(RandomLookAroundGoal::default()));

            target_selector.add_goal(
                1,
                Box::new(
                    RevengeGoal::new(true).alerting_others_except(|entity_type| {
                        entity_type == &EntityType::ZOMBIFIED_PIGLIN
                    }),
                ),
            );
            target_selector.add_goal(
                2,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::PLAYER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::VILLAGER, true),
            );
            target_selector.add_goal(
                3,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::IRON_GOLEM, true),
            );
            target_selector.add_goal(
                5,
                ActiveTargetGoal::with_default(&mob_arc.mob_entity, &EntityType::TURTLE, true),
            );
        };

        mob_arc
    }

    #[must_use]
    pub fn can_break_doors(&self) -> bool {
        self.can_break_doors.load(Ordering::Relaxed)
    }

    pub fn set_can_break_doors(&self, can_break_doors: bool, mob: &dyn Mob) {
        if self
            .can_break_doors
            .swap(can_break_doors, Ordering::Relaxed)
            != can_break_doors
        {
            let mut stopped = {
                let mut goal_selector = self
                    .mob_entity
                    .goals_selector
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if can_break_doors {
                    goal_selector.add_goal(1, Box::new(BreakDoorGoal::default()));
                    Vec::new()
                } else {
                    goal_selector.remove_goals::<BreakDoorGoal>()
                }
            };
            for goal in &mut stopped {
                goal.stop(mob);
            }
        }
    }
}

impl ZombieEntityBase {
    /// Updates zombie baby state, movement speed and collision dimensions.
    pub fn set_baby(&self, baby: bool) {
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
            apply_baby_speed_modifier(speed, baby);
        }
        // Zombie.getDefaultDimensions: BABY_DIMENSIONS (Zombie.java:91), independent of age ticking.
        let entity = &living.entity;
        let dimensions = if baby {
            pumpkin_util::math::boundingbox::EntityDimensions::new(0.49, 0.98, 0.775)
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

    #[must_use]
    pub fn is_baby(&self) -> bool {
        self.is_baby.load(Ordering::Relaxed)
    }

    /// Runs Zombie.finalizeSpawn, with this subtype's door-breaking capability.
    pub fn finalize_zombie_spawn(
        &self,
        caller: &dyn Mob,
        entity: &Arc<dyn crate::entity::EntityBase>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &RegionalDifficulty,
        reason: super::spawn::SpawnReason,
        group_data: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        Some(finalize::finalize_spawn(
            caller,
            entity,
            view,
            difficulty,
            reason,
            group_data,
            |enabled| {
                let enabled = enabled && caller.get_entity().entity_type != &EntityType::DROWNED;
                self.set_can_break_doors(enabled, caller);
                self.mob_entity
                    .navigator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .set_can_open_doors(enabled);
            },
        ))
    }
}

pub(super) fn apply_baby_speed_modifier(
    speed: &mut crate::entity::attributes::AttributeInstance,
    baby: bool,
) {
    // Zombie.setBaby / SPEED_MODIFIER_BABY: +50% of base speed, never serialized.
    speed.remove_modifier("minecraft:baby");
    if baby {
        speed.add_or_replace_modifier(crate::entity::attributes::Modifier {
            id: "minecraft:baby".to_string(),
            amount: 0.5,
            operation: crate::entity::attributes::ModifierOperation::MultiplyBase,
            permanent: false,
        });
    }
}

impl Mob for ZombieEntityBase {
    fn finalize_spawn_with_context(
        &self,
        entity: &Arc<dyn crate::entity::EntityBase>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        difficulty: &RegionalDifficulty,
        reason: super::spawn::SpawnReason,
        group_data: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        self.finalize_zombie_spawn(self, entity, view, difficulty, reason, group_data)
    }
    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }

    fn get_base_experience_reward(&self) -> u32 {
        // Zombie.getBaseExperienceReward multiplies the base before Mob adds equipment XP.
        let base = self
            .mob_entity
            .living_entity
            .entity
            .entity_type
            .experience_reward;
        super::equipped_mob_experience(
            &self.mob_entity.living_entity,
            zombie_experience_base(base, self.is_baby()),
        )
    }

    fn spawn_as_baby(&self) -> bool {
        self.set_baby(true);
        true
    }

    fn populate_default_equipment_slots(
        &self,
        _world: &Arc<World>,
        difficulty: &RegionalDifficulty,
    ) {
        // Default armor slots (super.populateDefaultEquipmentSlots)
        if rand::random::<f32>()
            < MobEntity::MAX_WEARING_ARMOR_CHANCE * difficulty.special_multiplier
        {
            let mut armor_type = rand::random_range(0..3);
            for _ in 1..=3 {
                if rand::random::<f32>() < MobEntity::WEARING_ARMOR_UPGRADE_MATERIAL_CHANCE {
                    armor_type += 1;
                }
            }

            let partial_chance = if difficulty.base_difficulty == Difficulty::Hard {
                0.1f32
            } else {
                0.25f32
            };

            let living = &self.mob_entity.living_entity;
            let mut equipment = living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut first = true;

            for slot in &MobEntity::EQUIPMENT_POPULATION_ORDER {
                let current = equipment.get(slot);
                if !first && rand::random::<f32>() < partial_chance {
                    break;
                }
                first = false;
                if current.is_empty()
                    && let Some(item) = MobEntity::get_equipment_for_slot(slot, armor_type)
                {
                    equipment.put(slot, ItemStack::new(1, item));
                }
            }
        }

        let weapon_chance = if difficulty.base_difficulty == Difficulty::Hard {
            0.05f32
        } else {
            0.01f32
        };
        if rand::random::<f32>() < weapon_chance {
            let r = rand::random_range(0..6);
            let weapon_item = match r {
                0 => &Item::IRON_SWORD,
                1 => &Item::IRON_SPEAR,
                _ => &Item::IRON_SHOVEL,
            };
            let living = &self.mob_entity.living_entity;
            let mut equipment = living
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            equipment.put(&EquipmentSlot::MAIN_HAND, ItemStack::new(1, weapon_item));
        }
    }

    fn mob_write_nbt(&self, nbt: &mut NbtCompound) {
        // Vanilla Zombie.addAdditionalSaveData; shared by every zombie-family mob.
        nbt.put_bool("IsBaby", self.is_baby());
        if self.can_break_doors() {
            nbt.put_bool("CanBreakDoors", true);
        }
    }

    fn mob_read_nbt(&self, nbt: &NbtCompound) {
        self.set_baby(nbt.get_bool("IsBaby").unwrap_or(false));
        if let Some(can_break_doors) = nbt.get_bool("CanBreakDoors") {
            self.set_can_break_doors(can_break_doors, self);
        }
    }
}

pub(super) fn zombie_experience_base(base: u32, baby: bool) -> u32 {
    // Zombie.getBaseExperienceReward hardcodes the baby multiplier as 2.5.
    if baby {
        (f64::from(base) * 2.5) as u32
    } else {
        base
    }
}

#[cfg(test)]
mod death_experience_tests {
    #[test]
    fn baby_zombie_experience_truncates_before_equipment_bonus() {
        assert_eq!(super::zombie_experience_base(5, true), 12);
        assert_eq!(super::zombie_experience_base(5, false), 5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baby_speed_is_transient_and_removed_when_becoming_adult() {
        let mut speed = crate::entity::attributes::AttributeInstance::new(0.2);
        apply_baby_speed_modifier(&mut speed, true);
        apply_baby_speed_modifier(&mut speed, true);
        assert!((speed.value() - 0.3).abs() < f64::EPSILON);
        assert!(speed.pack().is_empty());
        apply_baby_speed_modifier(&mut speed, false);
        assert!((speed.value() - 0.2).abs() < f64::EPSILON);
    }
}
