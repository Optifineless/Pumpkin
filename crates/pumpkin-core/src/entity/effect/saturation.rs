use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct SaturationMobEffect;

impl MobEffect for SaturationMobEffect {
    fn should_apply_effect_tick(&self, _duration: i32, _amplifier: u8) -> bool {
        true
    }

    fn apply_effect_tick(&self, living: &LivingEntity, amplifier: u8) -> bool {
        let world = living.entity.world.load();
        if let Some(entity) = world.get_entity_by_id(living.entity.entity_id)
            && let Some(player) = entity.get_player()
        {
            // FoodData.add caps food at 20, so saturating this u8 increment is equivalent.
            let hunger = amplifier.saturating_add(1);
            player.hunger_manager.add_hunger(hunger);
            player
                .hunger_manager
                .add_saturation((f32::from(amplifier) + 1.0) * 2.0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::java::combat_test_support::TestPlayer;
    use crate::server::combat_test_support::{server, world};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn saturation_clamps_normal_and_maximum_effect_levels() {
        let directory = tempfile::tempdir().unwrap();
        let server = server(directory.path());
        let world = world(&server, directory.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let effect = SaturationMobEffect;

        for (amplifier, food, saturation, expected_food, expected_saturation) in [
            (255, 0, 0.0, 20, 20.0),
            (235, 20, 0.0, 20, 20.0),
            (0, 5, 1.0, 6, 3.0),
            (0, 0, 0.0, 1, 1.0),
            (1, 19, 0.0, 20, 4.0),
        ] {
            player.hunger_manager.set_level(food);
            player.hunger_manager.set_saturation(saturation);
            assert!(effect.apply_effect_tick(&player.living_entity, amplifier));
            assert_eq!(player.hunger_manager.level.load(), expected_food);
            assert_eq!(player.hunger_manager.saturation.load(), expected_saturation);
        }
        crate::server::fixture_lifecycle::finish().await;
    }
}
