use std::sync::Arc;

use uuid::Uuid;

use pumpkin_data::entity::EntityStatus;
use rand::RngExt;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::{EntityBase, ai::pathfinder::NavigatorGoal, mob::Mob, r#type::from_type};

use super::{Controls, Goal};

pub struct BreedGoal {
    speed: f64,
    mate: Option<Arc<dyn EntityBase>>,
    timer: i32,
}

impl BreedGoal {
    #[must_use]
    pub fn new(speed: f64) -> Box<Self> {
        Box::new(Self {
            speed,
            mate: None,
            timer: 0,
        })
    }

    fn find_mate(mob: &dyn Mob) -> Option<Arc<dyn EntityBase>> {
        let mob_entity = mob.get_mob_entity();
        if !mob_entity.is_in_love() {
            return None;
        }

        let entity = mob.get_entity();
        let pos = entity.pos.load();
        let world = entity.world.load();
        let my_type = entity.entity_type;
        let my_uuid = entity.entity_uuid;

        let nearby = world.get_nearby_entities(pos, 8.0);
        let mut closest: Option<(f64, Arc<dyn EntityBase>)> = None;

        for candidate in nearby.values() {
            let c_entity = candidate.get_entity();
            if c_entity.entity_uuid == my_uuid {
                continue;
            }
            if c_entity.entity_type != my_type {
                continue;
            }
            if !candidate.is_in_love() || !candidate.is_breeding_ready() || candidate.is_panicking()
            {
                continue;
            }

            let dist = pos.squared_distance_to_vec(&c_entity.pos.load());
            match &closest {
                Some((best_dist, _)) if dist >= *best_dist => {}
                _ => closest = Some((dist, candidate.clone())),
            }
        }

        closest.map(|(_, e)| e)
    }

    fn breed(mob: &dyn Mob, mate: &dyn EntityBase) {
        let mob_entity = mob.get_mob_entity();
        let entity = mob.get_entity();
        let world = entity.world.load();

        let player_opt = mob_entity
            .breeder
            .load()
            .and_then(|uuid| world.get_player_by_uuid(uuid));
        if let Some(player) = player_opt {
            let entity_type_name = entity.entity_type.resource_name;
            player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::AnimalsBred as i32,
                1,
            );

            player.trigger_advancement_criterion(
                pumpkin_data::advancement::Advancement::HUSBANDRY_BREED_AN_ANIMAL,
                "bred",
            );
            player.trigger_advancement_criterion(
                pumpkin_data::advancement::Advancement::HUSBANDRY_BRED_ALL_ANIMALS,
                &format!("minecraft:{entity_type_name}"),
            );
        }

        mob_entity.reset_love_ticks();
        mob_entity
            .breeding_cooldown
            .store(6000, std::sync::atomic::Ordering::Relaxed);

        mate.reset_love();
        mate.set_breeding_cooldown(6000);

        let parent_pos = entity.pos.load();
        let baby = from_type(entity.entity_type, parent_pos, &world, Uuid::new_v4());
        crate::entity::mob::spawn::inherit_breeding_variant(mob, mate, baby.as_ref());
        baby.get_entity().set_age(-24000);
        let world_full = entity.world.load_full();
        world_full.spawn_entity(baby);

        world_full.send_entity_status(entity, EntityStatus::InLoveHearts, None);
        // Animal.finalizeSpawnChildFromBreeding (also Fox/Turtle): one raw-value orb at the
        // parent, only while the mobDrops game rule allows it.
        if world_full.level_info.load().game_rules.mob_drops {
            ExperienceOrbEntity::spawn_single(
                &world_full,
                parent_pos,
                mob.get_random().random_range(1..8),
            );
        }
    }
}

impl Goal for BreedGoal {
    fn can_start(&mut self, mob: &dyn Mob) -> bool {
        let mob_entity = mob.get_mob_entity();
        if !mob_entity.is_breeding_ready() || !mob_entity.is_in_love() {
            return false;
        }

        self.mate = Self::find_mate(mob);
        self.mate.is_some()
    }

    fn should_continue(&mut self, _mob: &dyn Mob) -> bool {
        let Some(mate) = &self.mate else {
            return false;
        };

        if !mate.get_entity().is_alive() || mate.is_panicking() {
            return false;
        }

        mate.is_in_love() && self.timer < 60
    }

    fn start(&mut self, _mob: &dyn Mob) {
        self.timer = 0;
    }

    fn stop(&mut self, mob: &dyn Mob) {
        self.mate = None;
        self.timer = 0;
        let mut navigator = mob
            .get_mob_entity()
            .navigator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        navigator.stop();
    }

    fn tick(&mut self, mob: &dyn Mob) {
        let Some(mate) = &self.mate else {
            return;
        };

        let mob_entity = mob.get_mob_entity();
        let mate_pos = mate.get_entity().pos.load();

        {
            let mut look_control = mob_entity
                .look_control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            look_control.look_at_entity(mob, mate);
        };

        let my_pos = mob.get_entity().pos.load();
        let dist_sq = my_pos.squared_distance_to_vec(&mate_pos);

        {
            let mut navigator = mob_entity
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            navigator.set_progress(NavigatorGoal::new(my_pos, mate_pos, self.speed));
        };

        self.timer += 1;

        if self.timer >= self.get_tick_count(60) && dist_sq < 9.0 {
            Self::breed(mob, mate.as_ref());
        }
    }

    fn controls(&self) -> Controls {
        Controls::MOVE | Controls::LOOK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::spawn_test_support::Fixture;
    use pumpkin_data::entity::EntityType;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_util::math::vector3::Vector3;

    #[tokio::test]
    async fn breeding_drops_one_raw_value_orb_at_the_parent() {
        let fixture = Fixture::new();
        let pos = Vector3::new(8.0, 100.0, 8.0);
        let parent = from_type(&EntityType::COW, pos, &fixture.world, Uuid::new_v4());
        let partner = from_type(&EntityType::COW, pos, &fixture.world, Uuid::new_v4());
        for _ in 0..32 {
            fixture.world.entities.store(Arc::new(Vec::new()));
            BreedGoal::breed(parent.get_mob().unwrap(), partner.as_ref());
            let entities = fixture.world.entities.load();
            let orbs: Vec<_> = entities
                .iter()
                .filter_map(|entity| entity.cast_any().downcast_ref::<ExperienceOrbEntity>())
                .collect();
            assert_eq!(orbs.len(), 1);
            assert!((1..=7).contains(&orbs[0].get_value()));
            assert_eq!(orbs[0].get_entity().pos.load(), pos);
        }
        fixture.finish().await;
        crate::server::fixture_lifecycle::finish().await;
    }

    #[tokio::test]
    async fn breeding_keeps_parent_variants_after_constructor_defers_selection() {
        let fixture = Fixture::new();
        for ty in [&EntityType::COW, &EntityType::PIG, &EntityType::CHICKEN] {
            let parent = from_type(
                ty,
                Vector3::new(8.5, 64.0, 8.5),
                &fixture.world,
                Uuid::new_v4(),
            );
            let partner = from_type(
                ty,
                Vector3::new(8.5, 64.0, 8.5),
                &fixture.world,
                Uuid::new_v4(),
            );
            parent.set_variant_name("minecraft:warm");
            partner.set_variant_name("minecraft:warm");
            BreedGoal::breed(parent.get_mob().unwrap(), partner.as_ref());
            let entities = fixture.world.entities.load();
            let baby = entities
                .iter()
                .find(|entity| entity.get_entity().entity_type == ty)
                .unwrap();
            let mut saved = NbtCompound::new();
            baby.write_nbt(&mut saved);
            assert_eq!(saved.get_string("variant"), Some("minecraft:warm"));
        }
        fixture.finish().await;
        crate::server::fixture_lifecycle::finish().await;
    }
}
