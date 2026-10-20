#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real fluid regressions inspect successful fixture state"
)]

use super::b1020d_g06_test_support::{Action, Fixture, Observed, Recorder, Stage, boat, packets};
use crate::{
    entity::RemovalReason,
    plugin::{BoxFuture, EventHandler, EventPriority, entity::EntityCombustEvent},
    server::Server,
};
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType, item::Item};
use pumpkin_util::math::position::BlockPos;
use std::sync::{Arc, Mutex, atomic::Ordering};

#[derive(Default)]
struct Ignition(Mutex<Vec<(i32, f32)>>);

impl EventHandler<EntityCombustEvent> for Ignition {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityCombustEvent,
    ) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push((event.entity_id, event.duration_secs));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_boat_lava_contact_ticks_reach_damage() {
    for (kind, item) in [
        (&EntityType::OAK_BOAT, &Item::OAK_BOAT),
        (&EntityType::BAMBOO_RAFT, &Item::BAMBOO_RAFT),
        (&EntityType::OAK_CHEST_BOAT, &Item::OAK_CHEST_BOAT),
        (&EntityType::BAMBOO_CHEST_RAFT, &Item::BAMBOO_CHEST_RAFT),
    ] {
        let fixture = Fixture::new();
        fixture.fill_contact(&Block::LAVA);
        let vessel = fixture.boat(kind);
        let observed = Recorder::register(&fixture.server);
        let ignition = Arc::new(Ignition::default());
        fixture
            .server
            .plugin_manager
            .register::<EntityCombustEvent, _>(ignition.clone(), EventPriority::Normal, true);
        fixture.tick(1);
        assert_eq!(
            vessel.get_entity().age.load(Ordering::Relaxed),
            1,
            "normal world tick must actually reach this boat"
        );
        assert_eq!(
            observed.damage_count(),
            1,
            "loaded lava must reach the real 4.0 vehicle damage boundary exactly once per tick"
        );
        assert_eq!(
            *observed.observed.lock().unwrap(),
            vec![Observed::Update, Observed::Damage(4.0, None)]
        );
        assert_eq!(
            *ignition.0.lock().unwrap(),
            vec![(vessel.get_entity().entity_id, 15.0)],
            "contact must use the real lava ignition effect"
        );
        assert!(vessel.get_entity().touching_lava.load(Ordering::Relaxed));
        assert!(vessel.get_entity().fire_ticks.load(Ordering::Relaxed) > 0);
        assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), 40.0);
        assert!(vessel.get_entity().is_alive());
        fixture.tick(1);
        assert_eq!(observed.damage_count(), 2);
        assert_eq!(observed.destroy_count(), 1);
        assert_eq!(
            boat(vessel.as_ref()).vehicle.get_damage(),
            79.0,
            "vehicle wobble must decay before this tick's lava damage"
        );
        assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
        fixture.assert_named_drop(item);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_air_water_and_fire_immunity_are_unaffected_controls() {
    for (block, immune) in [
        (&Block::AIR, false),
        (&Block::WATER, false),
        (&Block::LAVA, true),
    ] {
        let fixture = Fixture::new();
        fixture.fill_contact(block);
        let vessel = fixture.boat(&EntityType::OAK_BOAT);
        vessel
            .get_entity()
            .fire_immune
            .store(immune, Ordering::Relaxed);
        let observed = Recorder::register(&fixture.server);
        fixture.tick(3);
        assert_eq!(vessel.get_entity().age.load(Ordering::Relaxed), 3);
        assert!(vessel.get_entity().is_alive());
        assert_eq!(observed.damage_count(), 0);
        assert_eq!(observed.destroy_count(), 0);
        assert!(fixture.drops().is_empty());
        assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), 0.0);
        assert!(vessel.get_entity().fire_ticks.load(Ordering::Relaxed) <= 0);
        assert!(!vessel.get_entity().is_on_fire());
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_cancelled_lava_damage_observes_boundary_and_leaves_live_boat() {
    let fixture = Fixture::new();
    fixture.fill_contact(&Block::LAVA);
    let vessel = fixture.boat(&EntityType::BAMBOO_CHEST_RAFT);
    let observed = Recorder::register(&fixture.server);
    observed.cancel_damage.store(true, Ordering::Relaxed);
    fixture.tick(3);
    assert_eq!(
        observed.damage_count(),
        3,
        "cancellation control must observe the actual contact/damage boundary"
    );
    assert_eq!(observed.destroy_count(), 0);
    assert!(vessel.get_entity().is_alive());
    assert!(vessel.get_entity().fire_ticks.load(Ordering::Relaxed) > 0);
    assert_eq!(boat(vessel.as_ref()).vehicle.get_damage(), 0.0);
    assert!(fixture.drops().is_empty());
    observed.cancel_damage.store(false, Ordering::Relaxed);
    fixture.tick(2);
    assert_eq!(
        observed.destroy_count(),
        1,
        "later same-life accepted lava damage must still commit"
    );
    fixture.assert_named_drop(&Item::BAMBOO_CHEST_RAFT);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_base_tick_runs_once_and_normal_portal_contact_is_preserved() {
    let fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    vessel
        .get_entity()
        .portal_cooldown
        .store(7, Ordering::Relaxed);
    vessel
        .get_entity()
        .riding_cooldown
        .store(5, Ordering::Relaxed);
    fixture.tick(1);
    assert_eq!(
        vessel.get_entity().portal_cooldown.load(Ordering::Relaxed),
        6
    );
    assert_eq!(
        vessel.get_entity().riding_cooldown.load(Ordering::Relaxed),
        4
    );
    let destination = fixture.other_world(Dimension::THE_NETHER);
    vessel
        .get_entity()
        .portal_cooldown
        .store(0, Ordering::Relaxed);
    fixture.set_block(BlockPos::new(8, 64, 8), &Block::NETHER_PORTAL);
    fixture.tick(1);
    let (portal_destination, inside_portal) = {
        let manager = vessel.get_entity().portal_manager.lock().unwrap();
        let processor = manager
            .as_ref()
            .expect("real portal block collision must retain normal boat portal admission");
        (
            processor.destination_world.clone(),
            processor.inside_portal_this_tick,
        )
    };
    assert!(Arc::ptr_eq(&portal_destination, &destination));
    assert!(inside_portal);
    assert!(vessel.get_entity().is_alive());
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_lava_callback_removal_cannot_publish_retired_base_effects() {
    let mut fixture = Fixture::new();
    fixture.fill_contact(&Block::LAVA);
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    let observed = Recorder::register(&fixture.server);
    vessel
        .get_entity()
        .riding_cooldown
        .store(5, Ordering::Relaxed);
    vessel.get_entity().send_dirty_entity_data();
    fixture.player.take_packets();
    let retired = Arc::new(Mutex::new(None));
    let snapshot = retired.clone();
    let weak = Arc::downgrade(&vessel);
    Action::register(&fixture.server, Stage::Damage, move || {
        let vessel = weak.upgrade().unwrap();
        let entity = vessel.get_entity();
        entity.remove();
        *snapshot.lock().unwrap() = Some((
            entity.has_visual_fire.load(Ordering::Relaxed),
            entity.flags.load(Ordering::Relaxed),
            entity.bedrock_flags.load(Ordering::Relaxed),
            entity.bedrock_flags_two.load(Ordering::Relaxed),
            entity.riding_cooldown.load(Ordering::Relaxed),
        ));
    });
    fixture.tick(1);
    assert_eq!(
        observed.damage_count(),
        1,
        "real loaded lava must reach the removal callback"
    );
    assert_eq!(observed.destroy_count(), 0);
    assert!(vessel.get_entity().removal_reason.load() == Some(RemovalReason::Discarded));
    let state = retired.lock().unwrap().unwrap();
    let entity = vessel.get_entity();
    assert_eq!(
        entity.has_visual_fire.load(Ordering::Relaxed),
        state.0,
        "base tick published fire visualization after callback retirement"
    );
    assert_eq!(
        entity.flags.load(Ordering::Relaxed),
        state.1,
        "retired base tick changed Java metadata flags"
    );
    assert_eq!(
        entity.bedrock_flags.load(Ordering::Relaxed),
        state.2,
        "retired base tick changed Bedrock metadata flags"
    );
    assert_eq!(entity.bedrock_flags_two.load(Ordering::Relaxed), state.3);
    assert_eq!(entity.riding_cooldown.load(Ordering::Relaxed), state.4);
    assert!(
        !entity.synched_data.is_dirty(),
        "retired base tick queued stale metadata"
    );
    assert_eq!(packets(&mut fixture.player, entity.entity_id), (1, 0));
    assert!(fixture.drops().is_empty());
    fixture.finish().await;
}
