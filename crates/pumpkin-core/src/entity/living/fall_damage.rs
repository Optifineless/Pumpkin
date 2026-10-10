use super::{EntityBase, LivingEntity};
use pumpkin_data::{
    attributes::Attributes,
    damage::DamageType,
    statistic::{CustomStatistic, StatisticCategory},
};

impl LivingEntity {
    /// Applies `LivingEntity.causeFallDamage` with the landing block's damage source.
    pub fn handle_fall_damage_from(
        &self,
        caller: &dyn EntityBase,
        fall_distance: f32,
        damage_per_distance: f32,
        damage_type: DamageType,
    ) {
        let may_fly = caller.get_player().is_some_and(|player| {
            player
                .abilities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .allow_flying
        });
        if may_fly || self.is_immune_to_fall_damage() {
            return;
        }

        // Player.causeFallDamage awards distance before Player.isInvulnerableTo gates hurt.
        if fall_distance >= 2.0
            && let Some(player) = caller.get_player()
        {
            player.increment_stat(
                StatisticCategory::Custom,
                CustomStatistic::FallOneCm as i32,
                (fall_distance * 100.0).round() as i32,
            );
        }

        let fall_distance = self
            .impulse
            .effective_fall_distance(fall_distance, self.entity.pos.load().y);
        let safe_fall_distance = self.get_attribute_value(&Attributes::SAFE_FALL_DISTANCE);
        let unsafe_fall_distance = f64::from(fall_distance) + 1.0E-6 - safe_fall_distance;

        // LivingEntity.calculateFallDamage applies the entity's attribute after the block modifier.
        let multiplier = self.get_attribute_value(&Attributes::FALL_DAMAGE_MULTIPLIER);
        let damage =
            (unsafe_fall_distance * f64::from(damage_per_distance) * multiplier).floor() as f32;
        if damage > 0.0 {
            self.impulse.reset();
            // LivingEntity.causeFallDamage plays the damage-based sound even if hurt rejects it.
            self.entity.play_sound(Self::get_fall_sound(damage as i32));
            self.damage(caller, damage, damage_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::entity::death_test_world::DeathTestWorld;
    use crate::net::java::combat_test_support::TestPlayer;
    use pumpkin_data::{
        attributes::Attributes,
        damage::DamageType,
        entity::EntityType,
        packet::clientbound::play::{DAMAGE_EVENT, SOUND},
        sound::Sound,
        statistic::CustomStatistic,
    };
    use pumpkin_protocol::ser::NetworkReadExt;
    use std::sync::atomic::Ordering::Relaxed;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn falling_statistics_survive_disabled_fall_damage() {
        let fixture = DeathTestWorld::new().await;
        fixture.server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.fall_damage = false;
            info
        });
        let player = fixture.player("fall-statistic");
        let living = &player.living_entity;
        living.handle_fall_damage_from(player.as_ref(), 6.0, 1.0, DamageType::FALL);
        assert_eq!(player.get_custom_stat(CustomStatistic::FallOneCm), 600);
        assert_eq!(living.health.load(), 20.0);

        living.handle_fall_damage_from(player.as_ref(), 1.99, 1.0, DamageType::FALL);
        player.abilities.lock().unwrap().allow_flying = true;
        living.handle_fall_damage_from(player.as_ref(), 6.0, 1.0, DamageType::FALL);
        assert_eq!(player.get_custom_stat(CustomStatistic::FallOneCm), 600);
        fixture.server.shutdown().await;
    }

    fn feedback_packets(recipient: &mut TestPlayer) -> Vec<(i32, Option<i32>)> {
        recipient
            .take_packets()
            .iter()
            .map(|packet| {
                let mut data = packet.as_ref();
                let id = data.get_var_int().unwrap().0;
                let sound = (id == SOUND.0).then(|| data.get_var_int().unwrap().0 - 1);
                (id, sound)
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn falling_sounds_use_computed_damage_before_hurt() {
        let fixture = DeathTestWorld::new().await;
        let mut recipient = TestPlayer::new(&fixture.world());
        let player = recipient.player.clone();
        for (distance, modifier, source, sound, health) in [
            (
                6.0,
                1.0,
                DamageType::FALL,
                Sound::EntityGenericSmallFall,
                17.0,
            ),
            (
                8.5,
                2.0,
                DamageType::STALAGMITE,
                Sound::EntityGenericBigFall,
                9.0,
            ),
        ] {
            let living = &player.living_entity;
            living.set_health(20.0);
            living.hurt_cooldown.store(0, Relaxed);
            feedback_packets(&mut recipient);
            living.handle_fall_damage_from(player.as_ref(), distance, modifier, source);
            let packets = feedback_packets(&mut recipient);
            assert_eq!(packets.first(), Some(&(SOUND.0, Some(sound as i32))));
            assert!(packets.iter().any(|(id, _)| *id == DAMAGE_EVENT.0));
            assert_eq!(living.health.load(), health);
        }
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn falling_sounds_survive_rejected_damage() {
        let fixture = DeathTestWorld::new().await;
        let mut recipient = TestPlayer::new(&fixture.world());
        let player = recipient.player.clone();
        for fall_damage in [false, true] {
            fixture.server.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.fall_damage = fall_damage;
                info
            });
            player.abilities.lock().unwrap().invulnerable = fall_damage;
            feedback_packets(&mut recipient);
            player.living_entity.handle_fall_damage_from(
                player.as_ref(),
                6.0,
                1.0,
                DamageType::FALL,
            );
            assert_eq!(
                feedback_packets(&mut recipient),
                [(SOUND.0, Some(Sound::EntityGenericSmallFall as i32))]
            );
            assert_eq!(player.living_entity.health.load(), 20.0);
        }
        fixture.server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zero_fall_damage_multiplier_prevents_fall_and_stalagmite_damage() {
        let fixture = DeathTestWorld::new().await;
        for (multiplier, expected_health) in [(0.0, 20.0), (1.0, 8.0)] {
            for source in [DamageType::FALL, DamageType::STALAGMITE] {
                let cow = fixture.mob(&EntityType::COW);
                let living = cow.get_living_entity().unwrap();
                living.set_max_health(20.0);
                living.set_health(20.0);
                living.set_attribute_base(&Attributes::FALL_DAMAGE_MULTIPLIER, multiplier);
                living.handle_fall_damage_from(cow.as_ref(), 9.0, 2.0, source);
                assert_eq!(living.health.load(), expected_health);
            }
        }
        fixture.server.shutdown().await;
    }
}
