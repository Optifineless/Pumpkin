use crate::entity::effect::MobEffect;
use crate::entity::living::LivingEntity;

pub struct HungerMobEffect;

impl MobEffect for HungerMobEffect {
    // HungerMobEffect.shouldApplyEffectTickThisTick runs on every active tick.
    fn should_apply_effect_tick(&self, _duration: i32, _amplifier: u8) -> bool {
        true
    }

    fn apply_effect_tick(&self, living: &LivingEntity, amplifier: u8) -> bool {
        let world = living.entity.world.load();
        if let Some(entity) = world.get_entity_by_id(living.entity.entity_id)
            && let Some(player) = entity.get_player()
        {
            // HungerMobEffect.applyEffectTick calls Player.causeFoodExhaustion,
            // which ignores invulnerable players.
            let exhaustion = 0.005 * (f32::from(amplifier) + 1.0);
            player.add_exhaustion(exhaustion);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunger_ticks_between_twenty_tick_boundaries() {
        let effect = HungerMobEffect;
        for duration in [0, 1, 19, 20, 21] {
            assert!(effect.should_apply_effect_tick(duration, 0));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hunger_uses_per_tick_exhaustion_and_respects_invulnerability() {
        use crate::net::java::combat_test_support::TestPlayer;
        use crate::server::combat_test_support::{server, world};

        let directory = tempfile::tempdir().unwrap();
        let server = server(directory.path());
        let world = world(&server, directory.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let effect = HungerMobEffect;

        for (amplifier, expected) in [(0, 0.005), (1, 0.01), (255, 1.28)] {
            player.hunger_manager.set_exhaustion(0.0);
            assert!(effect.apply_effect_tick(&player.living_entity, amplifier));
            assert_eq!(player.hunger_manager.get_exhaustion(), expected);
        }

        player.abilities.lock().unwrap().invulnerable = true;
        player.hunger_manager.set_exhaustion(0.0);
        assert!(effect.apply_effect_tick(&player.living_entity, 255));
        assert_eq!(player.hunger_manager.get_exhaustion(), 0.0);
        crate::server::fixture_lifecycle::finish().await;
    }
}
