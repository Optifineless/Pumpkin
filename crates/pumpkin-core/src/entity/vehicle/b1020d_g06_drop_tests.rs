#![expect(
    clippy::unwrap_used,
    reason = "Real damage regressions inspect successful fixture state"
)]

use super::b1020d_g06_test_support::{Fixture, Observed, Recorder, boat, packets};
use crate::entity::RemovalReason;
use pumpkin_data::{damage::DamageType, entity::EntityType, item::Item};
use pumpkin_util::GameMode;
use std::sync::atomic::Ordering;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_boat_damage_drops_named_concrete_item_once() {
    for (kind, item) in [
        (&EntityType::OAK_BOAT, &Item::OAK_BOAT),
        (&EntityType::BAMBOO_RAFT, &Item::BAMBOO_RAFT),
        (&EntityType::OAK_CHEST_BOAT, &Item::OAK_CHEST_BOAT),
        (&EntityType::BAMBOO_CHEST_RAFT, &Item::BAMBOO_CHEST_RAFT),
    ] {
        let mut fixture = Fixture::new();
        let vessel = fixture.boat(kind);
        let events = Recorder::register(&fixture.server);
        fixture.player.take_packets();
        // The real Java attack handler charges five full-strength empty-hand hits.
        for _ in 0..5 {
            fixture.attack(vessel.as_ref());
        }
        assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
        assert!(
            fixture
                .world
                .get_entity_by_uuid(vessel.get_entity().entity_uuid)
                .is_none()
        );
        assert!(
            fixture
                .world
                .entity_tracker
                .get_tracked_entity(vessel.get_entity().entity_id)
                .is_none()
        );
        fixture.assert_named_drop(item);
        assert_eq!(events.damage_count(), 5);
        assert_eq!(
            events.destroy_count(),
            1,
            "one destruction callback sequence must own the item"
        );
        let attacker = fixture.player.player.entity_id();
        assert_eq!(
            events.observed.lock().unwrap().last(),
            Some(&Observed::Destroy(Some(attacker)))
        );
        assert_eq!(
            packets(&mut fixture.player, vessel.get_entity().entity_id).0,
            1,
            "first-removal cleanup must announce the boat once"
        );
        assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
        fixture.attack(vessel.as_ref());
        fixture.assert_named_drop(item);
        assert_eq!(events.destroy_count(), 1);
        assert_eq!(
            packets(&mut fixture.player, vessel.get_entity().entity_id).0,
            0,
            "repeat lethal calls must not repeat cleanup"
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_creative_and_entity_drops_rule_suppress_only_vehicle_item() {
    for creative in [false, true] {
        let mut fixture = Fixture::new();
        if creative {
            assert!(fixture.player.player.set_gamemode(GameMode::Creative));
        } else {
            fixture.world.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.entity_drops = false;
                info
            });
        }
        let vessel = fixture.boat(&EntityType::OAK_CHEST_BOAT);
        let events = Recorder::register(&fixture.server);
        fixture.player.take_packets();
        for _ in 0..5 {
            fixture.attack(vessel.as_ref());
        }
        assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
        assert!(
            fixture.drops().is_empty(),
            "Creative/rule suppression must not publish a vehicle item"
        );
        assert_eq!(events.destroy_count(), 1);
        assert_eq!(events.damage_count(), if creative { 1 } else { 5 });
        assert_eq!(
            packets(&mut fixture.player, vessel.get_entity().entity_id).0,
            1
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_cancelled_and_nonlethal_damage_keep_existing_wobble_timing() {
    for stage in 0..3 {
        let fixture = Fixture::new();
        let vessel = fixture.boat(&EntityType::BAMBOO_RAFT);
        let events = Recorder::register(&fixture.server);
        events.cancel_damage.store(stage == 1, Ordering::Relaxed);
        events.cancel_destroy.store(stage == 2, Ordering::Relaxed);
        let amount = if stage == 0 { 1.0 } else { 5.0 };
        let accepted = vessel.damage(vessel.as_ref(), amount, DamageType::GENERIC);
        assert_eq!(accepted, stage == 0);
        assert!(vessel.get_entity().is_alive());
        assert!(vessel.get_entity().removal_reason.load().is_none());
        assert!(fixture.drops().is_empty());
        let expected = if stage == 1 { 0.0 } else { amount * 10.0 };
        assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), expected);
        assert_eq!(
            boat(vessel.as_ref()).vehicle.get_hurt_time(),
            if stage == 1 { 0 } else { 10 }
        );
        assert_eq!(
            boat(vessel.as_ref()).vehicle.get_hurt_dir(),
            if stage == 1 { 1 } else { -1 }
        );
        fixture.tick(1);
        assert_eq!(
            boat(vessel.as_ref()).vehicle.get_damage(),
            (expected - 1.0).max(0.0)
        );
        assert_eq!(
            boat(vessel.as_ref()).vehicle.get_hurt_time(),
            if stage == 1 { 0 } else { 9 }
        );
        assert_eq!(events.destroy_count(), usize::from(stage == 2));
        assert_eq!(
            events.observed.lock().unwrap().last(),
            Some(&Observed::Update)
        );
        // Cancellation releases logical admission; a later same-life hit can commit.
        events.cancel_damage.store(false, Ordering::Relaxed);
        events.cancel_destroy.store(false, Ordering::Relaxed);
        assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
        fixture.assert_named_drop(&Item::BAMBOO_RAFT);
        fixture.finish().await;
    }
}
