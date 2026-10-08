use super::super::*;
use crate::entity::death_test_world::DeathTestWorld;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat fixtures and packet reads must be valid"
)]
async fn orchestration_review_spear_accounts_damage_and_restores_player_motion() {
    use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
    use crate::{
        net::java::combat_test_support::TestPlayer,
        server::combat_test_support::{server, world},
    };
    use pumpkin_protocol::ser::NetworkReadExt;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    let mut target = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![player.clone(), target.player.clone()]));
    player.last_attacked_ticks.store(100, Ordering::Relaxed);
    let old = Vector3::new(0.1, 0.0, 0.2);
    target.player.get_entity().velocity.store(old);
    target.take_packets();
    assert!(SpearItem::stab_attack(
        &player,
        &server,
        Hand::Right,
        &ItemStack::new(1, &Item::IRON_SPEAR),
        &(target.player.clone() as Arc<dyn EntityBase>),
        4.0,
        StabEffects {
            damage: true,
            knockback: true,
            dismount: false
        }
    ));
    assert_eq!(
        player.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageDealt as i32
        ),
        40
    );
    assert_eq!(target.player.get_entity().velocity.load(), old);
    assert_eq!(
        target
            .take_packets()
            .into_iter()
            .filter(|bytes| bytes.as_ref().get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION.0)
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_outgoing_spear_attack_memory_uses_living_ticks() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("Spearman");
    player.living_entity.tick(&*player, &fixture.server);
    let target = fixture.mob(&EntityType::COW);
    let stack = ItemStack::new(1, &Item::IRON_SPEAR);
    assert!(SpearItem::stab_attack(
        &player,
        &fixture.server,
        Hand::Right,
        &stack,
        &target,
        1.0,
        StabEffects {
            damage: true,
            knockback: false,
            dismount: false
        }
    ));
    assert_eq!(
        player
            .living_entity
            .last_attack_time
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        player
            .living_entity
            .last_attacking_id
            .load(Ordering::Relaxed),
        target.get_entity().entity_id
    );
}
