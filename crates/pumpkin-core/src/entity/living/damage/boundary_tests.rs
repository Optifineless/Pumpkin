use super::tests::{hit, packet_ids, raise};
use super::*;
use crate::{
    entity::player::statistics::{CustomStatistic, StatisticCategory},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
    world::scoreboard::{CollisionRule, NameTagVisibility, Team},
};
use pumpkin_data::{Advancement, entity::EntityType};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::text::{TextComponent, color::NamedColor};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_inactivity_resets_before_cooldown_and_peaceful_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let zombie = LivingEntity::new(crate::entity::Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ZOMBIE,
    ));
    // LivingEntity.hurtServer resets noActionTime even when cooldown rejects the hit.
    for accepted in [true, false] {
        zombie.no_action_time.store(601, Relaxed);
        assert_eq!(zombie.damage(&zombie, 2.0, DamageType::GENERIC), accepted);
        assert_eq!(zombie.no_action_time.load(Relaxed), 0);
    }
    let player = TestPlayer::new(&world).player;
    world.set_difficulty(pumpkin_util::Difficulty::Peaceful);
    player.living_entity.no_action_time.store(601, Relaxed);
    // Player.hurtServer resets inactivity before difficulty reduces damage to zero.
    assert!(!hit(&player, 6.0, DamageType::MOB_ATTACK, &zombie));
    assert_eq!(player.living_entity.no_action_time.load(Relaxed), 0);
    assert_eq!(player.living_entity.health.load(), 20.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_damage_packets_preserve_cause_and_direct_source_and_tilt_only_the_victim() {
    use pumpkin_data::packet::clientbound::play::{DAMAGE_EVENT, HURT_ANIMATION};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut victim = TestPlayer::new(&world);
    let mut watcher = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        victim.player.clone(),
        watcher.player.clone(),
    ]));
    let tracked = world
        .entity_tracker
        .get_tracked_entity(victim.player.entity_id())
        .unwrap();
    tracked.seen_by.insert(watcher.player.gameprofile.id);
    tracked.add_pairing(&watcher.player);
    packet_ids(&mut victim);
    packet_ids(&mut watcher);
    let arrow = crate::entity::Entity::new(world, Vector3::new(0.0, 0.0, 1.0), &EntityType::ARROW);
    arrow.velocity.store(Vector3::new(0.0, 0.0, -1.0));
    assert!(victim.player.damage_with_context(
        victim.player.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        Some(&arrow),
        Some(watcher.player.as_ref())
    ));
    let victim_id = victim.player.entity_id();
    let cause_id = watcher.player.entity_id();
    let mut victim_tilts = 0;
    let mut watcher_tilts = 0;
    for (fixture, tilts) in [
        (&mut victim, &mut victim_tilts),
        (&mut watcher, &mut watcher_tilts),
    ] {
        let mut damage_events = 0;
        for bytes in fixture.take_packets() {
            let mut data = bytes.as_ref();
            match data.get_var_int().unwrap().0 {
                id if id == DAMAGE_EVENT.0 => {
                    damage_events += 1;
                    assert_eq!(data.get_var_int().unwrap().0, victim_id);
                    assert_eq!(
                        data.get_var_int().unwrap().0,
                        i32::from(DamageType::ARROW.id)
                    );
                    assert_eq!(data.get_var_int().unwrap().0, cause_id + 1);
                    assert_eq!(data.get_var_int().unwrap().0, arrow.entity_id + 1);
                    assert!(!data.get_bool().unwrap());
                }
                id if id == HURT_ANIMATION.0 => {
                    *tilts += 1;
                    assert_eq!(data.get_var_int().unwrap().0, victim_id);
                    assert_eq!(data.get_f32().unwrap(), 90.0);
                }
                _ => {}
            }
        }
        assert_eq!(damage_events, 1);
    }
    assert_eq!(victim_tilts, 1);
    assert_eq!(watcher_tilts, 0);
    // Direct Entity.add_velocity delivers motion immediately in Pumpkin.
    victim
        .player
        .get_entity()
        .add_velocity(Vector3::new(0.0, 0.0, 0.5));
    assert_eq!(
        packet_ids(&mut victim)
            .iter()
            .filter(|id| **id == pumpkin_data::packet::clientbound::play::SET_ENTITY_MOTION.0)
            .count(),
        1
    );
    victim.player.living_entity.flush_player_motion();
    assert!(!victim.player.get_entity().hurt_marked.load(Relaxed));
    assert!(packet_ids(&mut victim).is_empty());
    victim.player.living_entity.hurt_cooldown.store(0, Relaxed);
    assert!(victim.player.damage_with_context(
        victim.player.as_ref(),
        1.0,
        DamageType::EXPLOSION,
        None,
        None,
        None
    ));
    packet_ids(&mut victim);
    victim.player.living_entity.flush_player_motion();
    assert!(packet_ids(&mut victim).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Combat regressions require valid fixtures"
)]
async fn orchestration_team_gate_and_blocked_arrow_criterion_run_at_the_player_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim.advancements.lock().unwrap().player = Arc::downgrade(&victim);
    let team = Team {
        name: "friends".into(),
        display_name: TextComponent::text("friends"),
        options: 0,
        nametag_visibility: NameTagVisibility::Always,
        collision_rule: CollisionRule::Always,
        color: NamedColor::White,
        player_prefix: TextComponent::text(""),
        player_suffix: TextComponent::text(""),
        players: vec![victim.gameprofile.name.clone()],
    };
    world
        .scoreboard
        .lock()
        .unwrap()
        .add_team(world.as_ref(), team.clone());
    assert!(!hit(&victim, 4.0, DamageType::FIREWORKS, victim.as_ref()));
    let mut team = team;
    team.options = 1;
    world
        .scoreboard
        .lock()
        .unwrap()
        .update_team(world.as_ref(), team);
    assert!(hit(&victim, 4.0, DamageType::FIREWORKS, victim.as_ref()));
    victim.living_entity.hurt_cooldown.store(0, Relaxed);
    raise(&victim, 1.0);
    let arrow = crate::entity::Entity::new(world, Vector3::new(0.0, 0.0, 1.0), &EntityType::ARROW);
    assert!(!hit(&victim, 4.0, DamageType::ARROW, &arrow));
    let advancements = victim.advancements.lock().unwrap();
    assert!(
        advancements
            .progress
            .map
            .get(Advancement::STORY_DEFLECT_ARROW)
            .unwrap()
            .is_done()
    );
    drop(advancements);
    assert_eq!(
        victim.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::DamageBlockedByShield as i32
        ),
        40
    );
}
