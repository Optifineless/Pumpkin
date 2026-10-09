use super::{cow::CowEntity, mooshroom::MooshroomEntity};
use crate::entity::{Entity, EntityBase, ageable::AgeableMob};
use pumpkin_data::{cow_variant::CowVariant, entity::EntityType};
use std::sync::{Arc, atomic::Ordering};

impl MooshroomEntity {
    /// Vanilla `Mob.convertTo`
    pub(super) fn convert_to_cow(&self) -> Arc<CowEntity> {
        let entity = self.get_entity();
        let cow = CowEntity::new(Entity::new(
            entity.world.load_full(),
            entity.pos.load(),
            &EntityType::COW,
        ));
        // No cow.finalize_spawn(...), vanilla omits this likely to stop a 'biomed' cow being created
        cow.set_variant(CowVariant::default());

        let cow_entity = cow.get_entity();
        cow_entity.set_rotation(entity.yaw.load(), entity.pitch.load());
        cow_entity.head_yaw.store(entity.head_yaw.load());
        cow_entity.body_yaw.store(entity.body_yaw.load());
        cow_entity.velocity.store(entity.velocity.load());
        cow_entity
            .on_ground
            .store(entity.on_ground.load(Ordering::Relaxed), Ordering::Relaxed);
        cow.mob_entity
            .living_entity
            .fall_distance
            .store(self.mob_entity.living_entity.fall_distance.load());

        self.copy_common_to_cow(&cow);

        cow
    }

    // ConversionType.convertCommon with keepEquipment=false and preserveCanPickUpLoot=false.
    fn copy_common_to_cow(&self, cow: &CowEntity) {
        let entity = self.get_entity();
        let cow_entity = cow.get_entity();
        cow.set_age(self.get_age());
        let (data, cow_data) = (self.get_ageable_data(), cow.get_ageable_data());
        cow_data
            .forced_age
            .store(data.forced_age.load(Ordering::Relaxed), Ordering::Relaxed);
        cow_data.forced_age_timer.store(
            data.forced_age_timer.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        cow.mob_entity
            .set_left_handed(self.mob_entity.is_left_handed());
        cow.mob_entity.living_entity.hurt_cooldown.store(
            self.mob_entity
                .living_entity
                .hurt_cooldown
                .load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        // ConversionType.SINGLE also preserves the hurt animation introduced by the combat lane.
        cow.mob_entity.living_entity.hurt_time.store(
            self.mob_entity
                .living_entity
                .hurt_time
                .load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        if entity.is_fall_flying() {
            cow_entity.set_fall_flying(true);
        }
        if entity.pose.load() == pumpkin_data::entity::EntityPose::Sleeping {
            cow_entity.set_pose(pumpkin_data::entity::EntityPose::Sleeping);
        }
        cow_entity
            .custom_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(
                &entity
                    .custom_data
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
        cow.mob_entity
            .living_entity
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .player_memory_time = self
            .mob_entity
            .living_entity
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .player_memory_time;
        // ConversionType.convertCommon's ANGRY_AT memory and sleeping block position are not represented here.
        cow.mob_entity.set_no_ai(self.mob_entity.is_no_ai());
        cow.mob_entity.persistence_required.store(
            self.mob_entity.persistence_required.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        if let Some(custom_name) = &**entity.custom_name.load() {
            cow_entity.set_custom_name(custom_name.clone());
        }
        cow_entity.set_custom_name_visible(entity.custom_name_visible.load(Ordering::Relaxed));
        cow_entity.set_on_fire(entity.is_on_fire());
        cow_entity.invulnerable.store(
            entity.invulnerable.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        cow_entity.set_has_no_gravity(entity.has_no_gravity());
        cow_entity.portal_cooldown.store(
            entity.portal_cooldown.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        cow_entity.set_silent(entity.is_silent());
        cow_entity
            .scoreboard_tags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(
                &entity
                    .scoreboard_tags
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
    }

    // ConversionType.SINGLE / convertCommon, after cancellable spawn and transform preflight.
    pub(super) fn finish_conversion(&self, cow: &Arc<CowEntity>) {
        let mooshroom_entity = self.get_entity();
        let world = mooshroom_entity.world.load();
        // These send packets about the cow, so they wait until clients know it exists.
        let cow_living = &cow.mob_entity.living_entity;
        cow_living.set_absorption(self.mob_entity.living_entity.get_absorption());
        let effects: Vec<_> = self
            .mob_entity
            .living_entity
            .active_effects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for effect in effects {
            cow_living.add_effect(effect);
        }
        let holder = mooshroom_entity
            .leashed_to
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(holder) = holder {
            cow_living.entity.leash_to(holder);
        }

        // Vanilla `ConversionType.SINGLE` moves the first passenger and the vehicle over to the cow.
        let cow_base: Arc<dyn EntityBase> = cow.clone();
        let passenger = mooshroom_entity
            .passengers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first()
            .cloned();
        if let Some(passenger) = passenger {
            mooshroom_entity.remove_passenger_sync(passenger.get_entity().entity_id);
            passenger
                .get_entity()
                .riding_cooldown
                .store(0, Ordering::Relaxed);
            cow_living.entity.add_passenger(cow_base.clone(), passenger);
        }
        if let Some(vehicle) = mooshroom_entity.get_vehicle() {
            vehicle
                .get_entity()
                .remove_passenger_sync(mooshroom_entity.entity_id);
            vehicle
                .get_entity()
                .add_passenger(vehicle.clone(), cow_base);
        }

        // Vanilla `convertCommon` moves the team over to the cow.
        if let Some(team_name) = self.get_team_name() {
            let mut scoreboard = world
                .scoreboard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            scoreboard.add_player_to_team(&*world, &team_name, cow.get_scoreboard_name());
            scoreboard.remove_player_from_team(&*world, &team_name, &self.get_scoreboard_name());
        }
    }
}

#[cfg(test)]
#[path = "mooshroom_conversion_tests.rs"]
mod tests;
