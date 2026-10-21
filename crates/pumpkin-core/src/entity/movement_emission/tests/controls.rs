use super::*;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_damage::EntityDamageEvent,
    },
    server::Server,
};
use pumpkin_data::{
    entity::EntityType,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_protocol::{java::server::play::SMoveVehicle, ser::NetworkReadExt};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_same_position_teleport_resets_transient_distance() {
    for correction in [false, true] {
        let s = scenario(true, true);
        position(&s, false, at(1.5), true);
        assert_eq!(counts(&s), (0, 0));
        if correction {
            s.owner.client().force_tp(&s.owner.player, at(1.5));
        } else {
            s.owner.player.request_teleport(at(1.5), 0.0, 0.0);
        }
        confirm(&s);
        position(&s, true, at(1.75), true);
        assert_eq!(
            counts(&s),
            (0, 0),
            "same-position teleport discards earlier distance"
        );
        position(&s, false, at(3.25), true);
        assert_eq!(
            counts(&s),
            (1, 1),
            "independent movement after teleport still crosses threshold"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_exact_accepted_position_across_origin() {
    let s = scenario(true, true);
    s.owner
        .player
        .get_entity()
        .set_pos(Vector3::new(-0.8, 65.0, 8.0));
    position(&s, false, Vector3::new(0.9, 65.0, 8.0), true);
    assert_eq!(
        counts(&s),
        (1, 1),
        "subtraction/addition must not reconstruct accepted destination"
    );
    assert_eq!(
        s.sink.lock().unwrap().forwarded[0].position,
        Vector3::new(0.9, 65.0, 8.0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_wall_collision_counts_only_committed_distance() {
    let s = scenario(true, true);
    for y in 65..68 {
        for z in 7..10 {
            place(&s, &BlockPos::new(10, y, z), Block::STONE.default_state.id);
        }
    }
    let cow = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(8.5, 65.0, 8.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    s.world.add_entity_silent(cow.clone());
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(4.0, -0.08, 0.0));
    let blocked = cow.get_entity().pos.load();
    assert!(
        blocked.x > 9.0 && blocked.x < 9.6,
        "real wall clips requested four blocks"
    );
    assert_eq!(
        counts(&s),
        (0, 0),
        "clipped distance remains below first threshold"
    );
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(4.0, -0.08, 0.0));
    assert_eq!(cow.get_entity().pos.load(), blocked);
    assert_eq!(
        counts(&s),
        (0, 0),
        "blocked displacement cannot accumulate requested motion"
    );
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(-1.75, -0.08, 0.0));
    assert_eq!(
        counts(&s),
        (1, 1),
        "accepted return travel crosses threshold once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_off_preserves_real_fall_statistics_and_sounds() {
    let mut controls = Vec::new();
    for enabled in [false, true] {
        let mut s = scenario(enabled, true);
        s.owner
            .player
            .get_entity()
            .set_pos(Vector3::new(8.0, 70.0, 8.0));
        position(&s, false, Vector3::new(8.0, 66.0, 8.0), false);
        position(&s, true, START, true);
        let health = s.owner.player.living_entity.health.load();
        assert_eq!(health, 18.0, "OFF retains five-block landing damage");
        let stat = s
            .owner
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Custom, CustomStatistic::FallOneCm as i32);
        assert_eq!(stat, 500, "OFF retains landing statistic");
        let sounds = s
            .owner
            .take_packets()
            .iter()
            .filter(|bytes| {
                let mut data = bytes.as_ref();
                data.get_var_int()
                    .is_ok_and(|id| id.0 == pumpkin_data::packet::clientbound::play::SOUND)
            })
            .count();
        assert!(sounds > 0, "real fall still publishes sounds");
        controls.push((health, stat, sounds));
        assert_eq!(
            counts(&s),
            (1, 1),
            "existing damage game event remains once"
        );
        assert_eq!(
            s.sink.lock().unwrap().plugins[0].key,
            "entity_damage",
            "vertical fall is not STEP"
        );
    }
    assert_eq!(
        controls[0], controls[1],
        "movement switch preserves original side effects"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_vehicle_pending_correction_is_silent() {
    let s = scenario(true, true);
    let boat = crate::entity::r#type::from_type(
        &EntityType::OAK_BOAT,
        START,
        &s.world,
        uuid::Uuid::new_v4(),
    );
    s.world.add_entity_silent(boat.clone());
    boat.get_entity()
        .add_passenger(boat.clone(), s.owner.player.clone());
    s.owner.client().force_tp(&s.owner.player, START);
    send(
        &s,
        &SMoveVehicle {
            x: 9.75,
            y: 65.0,
            z: 8.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        },
    );
    assert_eq!(
        counts(&s),
        (0, 0),
        "new vehicle notification excludes awaiting teleport"
    );
    assert_eq!(
        boat.get_entity().pos.load(),
        at(1.75),
        "inherited vehicle authority is unchanged"
    );
}

struct RemoveOnFall {
    entity: Arc<dyn EntityBase>,
    calls: Arc<AtomicUsize>,
}
impl EventHandler<EntityDamageEvent> for RemoveOnFall {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityDamageEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.entity.get_entity().entity_id {
            self.calls.fetch_add(1, Relaxed);
            self.entity.get_entity().remove();
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_fall_callback_removal_rejects_nonplayer_continuation() {
    let s = scenario(true, true);
    let cow = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(5.5, 70.0, 8.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    s.world.add_entity_silent(cow.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    s.server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(RemoveOnFall {
            entity: cow.clone(),
            calls: calls.clone(),
        }),
        EventPriority::High,
        true,
    );
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(0.0, -4.0, 0.0));
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(2.0, -1.5, 0.0));
    assert_eq!(
        calls.load(Relaxed),
        1,
        "real fall must invoke damage callback"
    );
    assert!(cow.get_entity().is_removed());
    assert_eq!(
        counts(&s),
        (0, 0),
        "removed source must not notify from committed old movement"
    );
}
