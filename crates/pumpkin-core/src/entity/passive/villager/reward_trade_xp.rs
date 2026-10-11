use super::VillagerEntity;
use crate::{
    entity::{EntityBase, experience_orb::ExperienceOrbEntity},
    world::World,
};
use pumpkin_data::{effect::StatusEffect, potion::Effect};
use rand::RngExt;
use std::sync::{Arc, atomic::Ordering};

// VillagerData.java:19, NEXT_LEVEL_XP_THRESHOLDS is a hardcoded Java constant.
const NEXT_LEVEL_XP_THRESHOLDS: [i32; 5] = [0, 10, 70, 150, 250];

impl VillagerEntity {
    // Villager.rewardTradeXp -> shouldIncreaseLevel / increaseMerchantCareer.
    pub(super) fn reward_trade_xp(
        &self,
        world: &Arc<World>,
        xp_gain: i32,
        reward_exp: bool,
    ) -> i32 {
        let mut pop_xp = 3 + rand::rng().random_range(0..4);
        let current_xp = self.xp.fetch_add(xp_gain, Ordering::Relaxed) + xp_gain;
        let mut data = *self
            .villager_data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let level = data.level.0;
        if (1..5).contains(&level) && current_xp >= NEXT_LEVEL_XP_THRESHOLDS[level as usize] {
            data.level.0 += 1;
            self.set_villager_data(data);
            self.add_trades(data.profession_enum(), data.level.0);
            self.mob_entity.living_entity.add_effect(Effect {
                effect_type: &StatusEffect::REGENERATION,
                duration: 200,
                amplifier: 0,
                ambient: false,
                show_particles: true,
                show_icon: true,
                blend: false,
            });
            pop_xp += 5;
        }
        if reward_exp {
            ExperienceOrbEntity::spawn_single(
                world,
                self.get_entity().pos.load().add_raw(0.0, 0.5, 0.0),
                pop_xp,
            );
        }
        current_xp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Entity;
    use crate::server::combat_test_support::{server, world};
    use pumpkin_data::{entity::EntityType, villager::VillagerProfession};
    use pumpkin_util::math::vector3::Vector3;
    use uuid::Uuid;

    #[tokio::test]
    async fn villager_trades_drop_one_orb_and_add_five_on_level_up() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let pos = Vector3::new(8.0, 100.0, 8.0);
        let villager = VillagerEntity::new(Entity::new(world.clone(), pos, &EntityType::VILLAGER));
        villager.generate_trades(VillagerProfession::Farmer, 1);
        villager.offers.lock().unwrap()[0].xp = 0;
        villager.complete_trade(0, &world, Uuid::nil());
        let entities = world.entities.load_full();
        assert_eq!(entities.len(), 1);
        let orb = entities[0]
            .cast_any()
            .downcast_ref::<ExperienceOrbEntity>()
            .unwrap();
        assert!((3..=6).contains(&orb.get_value()));
        assert_eq!(orb.get_entity().pos.load(), pos.add_raw(0.0, 0.5, 0.0));
        world.entities.store(Arc::new(Vec::new()));
        villager.offers.lock().unwrap()[0].xp = 10;
        villager.complete_trade(0, &world, Uuid::nil());
        let entities = world.entities.load_full();
        assert_eq!(entities.len(), 1);
        let orb = entities[0]
            .cast_any()
            .downcast_ref::<ExperienceOrbEntity>()
            .unwrap();
        assert!((8..=11).contains(&orb.get_value()));
        assert_eq!(villager.villager_data.lock().unwrap().level.0, 2);
        assert_eq!(villager.xp.load(Ordering::Relaxed), 10);
        crate::server::fixture_lifecycle::finish().await;
    }
}
