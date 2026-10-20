use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{
        entity::{entity_dismount::EntityDismountEvent, entity_tame::EntityTameEvent},
        vehicle::vehicle_exit::VehicleExitEvent,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};

type TameCallback = dyn Fn(&Arc<Server>, &mut EntityTameEvent) + Send + Sync;
struct TameControl {
    callback: Box<TameCallback>,
    calls: AtomicUsize,
}
impl EventHandler<EntityTameEvent> for TameControl {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntityTameEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.callback)(server, event);
        Box::pin(async {})
    }
}

fn control(
    scenario: &Scenario,
    callback: impl Fn(&Arc<Server>, &mut EntityTameEvent) + Send + Sync + 'static,
) -> Arc<TameControl> {
    let handler = Arc::new(TameControl {
        callback: Box::new(callback),
        calls: AtomicUsize::new(0),
    });
    scenario
        .server
        .plugin_manager
        .register::<EntityTameEvent, _>(handler.clone(), EventPriority::Normal, true);
    handler
}

#[tokio::test]
async fn feat_horses_s2_tame_event_cancel_reentry_and_stale_session() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseCallback").await;
        let world = owner.player.world();
        let entity = spawn(&world);
        horse(&entity).set_temper(5);
        interact(&scenario, &owner, &entity);
        let target = entity.clone();
        let calls = control(&scenario, move |_, event| {
            let horse = horse(&target);
            assert!(
                target.get_entity().passengers.try_lock().is_ok(),
                "event retained passenger guard"
            );
            assert_eq!(horse.get_temper(), 5, "event retained horse state guard");
            assert!(!horse.is_tame());
            assert_eq!(horse.get_owner(), None);
            let world = target.get_entity().world.load_full();
            let server = world.server.upgrade().unwrap();
            // Nested real entity tick: selector is detached and attempt reservation stays owned.
            target.tick(target.as_ref(), &server);
            event.cancelled = true;
        });
        owner.packets();
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert_eq!(
            calls.calls.load(Ordering::SeqCst),
            1,
            "reentry duplicated tame callback"
        );
        assert!(
            !horse(&entity).is_tame(),
            "cancelled tame event committed tame"
        );
        assert_eq!(horse(&entity).get_owner(), None);
        assert_eq!(
            horse(&entity).get_temper(),
            5,
            "cancelled successful roll must not increment temper"
        );
        assert_mounted(&owner, &entity);
        assert!(
            statuses(&owner, &entity).is_empty(),
            "cancelled success fell through to buck or hearts"
        );
        assert!(!tamed_advancement(&owner));
        opportunity(&scenario, &entity).await;
        assert_eq!(
            calls.calls.load(Ordering::SeqCst),
            2,
            "next independent attempt must remain possible"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn feat_horses_s2_tame_callback_invalidations_never_publish_stale_success() {
    for change in 0..9 {
        run(async {
            let scenario = Scenario::new().await;
            let owner = scenario.join(Uuid::new_v4(), "HorseStale").await;
            let entity = spawn(&owner.player.world());
            horse(&entity).set_temper(5);
            interact(&scenario, &owner, &entity);
            let target = entity.clone();
            let rider = owner.player.clone();
            let destination = scenario.world(&Dimension::THE_NETHER);
            let callback_owner = Uuid::new_v4();
            let calls = control(&scenario, move |_, _| {
                assert!(target.get_entity().passengers.try_lock().is_ok());
                assert_eq!(horse(&target).get_temper(), 5);
                match change {
                    0 => rider.world().remove_entity(target.as_ref()),
                    1 => target.get_entity().remove_passenger(rider.entity_id()),
                    2 => {
                        rider.get_entity().set_world(destination.clone());
                    }
                    3 => {
                        horse(&target).set_temper(91);
                        horse(&target).set_owner(Some(callback_owner));
                        horse(&target).set_tame(true);
                    }
                    4 => {
                        rider.client.java().unwrap().close();
                    }
                    5 => {
                        target.get_living_entity().unwrap().set_health(0.0);
                    }
                    6 => {
                        // Concrete replacement sharing UUID is never the old admitted horse.
                        let world = rider.world();
                        world.remove_entity(target.as_ref());
                        let replacement = crate::entity::r#type::from_type(
                            &EntityType::HORSE,
                            SPAWN,
                            &world,
                            target.get_entity().entity_uuid,
                        );
                        world.add_entity_silent(replacement);
                    }
                    7 => {
                        target.get_entity().remove_passenger(rider.entity_id());
                        target
                            .get_entity()
                            .add_passenger(target.clone(), rider.clone());
                    }
                    8 => {
                        let origin = rider.world();
                        rider.get_entity().set_world(destination.clone());
                        rider.get_entity().set_world(origin);
                    }
                    _ => panic!("unknown callback case"),
                }
            });
            owner.packets();
            script(&entity, 0, 4);
            opportunity(&scenario, &entity).await;
            assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                horse(&entity).get_owner(),
                (change == 3).then_some(callback_owner),
                "stale attempt overwrote callback owner ({change})"
            );
            assert_eq!(
                horse(&entity).is_tame(),
                change == 3,
                "stale attempt committed tame ({change})"
            );
            assert_eq!(
                horse(&entity).get_temper(),
                if change == 3 { 91 } else { 5 }
            );
            assert!(
                statuses(&owner, &entity).is_empty(),
                "invalidated old instance published success ({change})"
            );
            assert!(!tamed_advancement(&owner));
            if change == 2 {
                owner
                    .player
                    .get_entity()
                    .set_world(scenario.world(&Dimension::OVERWORLD));
            }
            scenario.stop().await;
        })
        .await;
    }
}

#[tokio::test]
async fn feat_horses_s2_same_uuid_rejoin_and_payload_redirect_are_revalidated() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseRejoin").await;
        let entity = spawn(&owner.player.world());
        horse(&entity).set_temper(5);
        interact(&scenario, &owner, &entity);
        let original = owner.player.clone();
        let handler = control(&scenario, move |server, event| {
            assert!(
                server.tick_gate.try_lock().is_err(),
                "normal tick must own the disconnect/login gate during callback"
            );
            if Arc::ptr_eq(&event.owner, &original) {
                // The tick gate serializes full connection cleanup after this event returns.
                original.client.java().unwrap().close();
            }
        });
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert!(
            !horse(&entity).is_tame(),
            "closed original session committed success"
        );
        assert_eq!(horse(&entity).get_owner(), None);
        assert_eq!(horse(&entity).get_temper(), 5);
        // Login itself ticks and may restore the saved mount. Reserve the next successful
        // RNG opportunity for the explicit fresh-session ride below.
        script(&entity, 1, 0);
        scenario.leave(&owner).await;
        let new_session = scenario
            .join(owner.player.gameprofile.id, "HorseRejoin")
            .await;
        assert!(!Arc::ptr_eq(&new_session.player, &owner.player));
        let current = new_session
            .player
            .world()
            .get_entity_by_uuid(entity.get_entity().entity_uuid)
            .unwrap();
        assert!(
            Arc::ptr_eq(&current, &entity),
            "login must retain the scripted horse"
        );
        assert!(
            !horse(&current).is_tame(),
            "rejoined UUID inherited stale success"
        );
        assert_eq!(horse(&current).get_owner(), None);
        assert_eq!(horse(&current).get_temper(), 5);
        assert!(!tamed_advancement(&new_session));
        assert!(!statuses(&new_session, &current).contains(&7));
        if !new_session.player.get_entity().has_vehicle() {
            interact(&scenario, &new_session, &current);
        }
        assert_mounted(&new_session, &current);
        assert_eq!(current.get_entity().passengers.lock().unwrap().len(), 1);
        script(&current, 0, 4);
        opportunity(&scenario, &current).await;
        assert!(
            horse(&current).is_tame(),
            "fresh rejoined session must admit its own ride"
        );
        scenario.stop().await;
    })
    .await;

    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "HorseOriginal").await;
        let other = scenario.join(Uuid::new_v4(), "HorseOther").await;
        let entity = spawn(&owner.player.world());
        horse(&entity).set_temper(5);
        interact(&scenario, &owner, &entity);
        let redirected = other.player.clone();
        control(&scenario, move |_, event| {
            event.owner = redirected.clone();
        });
        script(&entity, 0, 4);
        opportunity(&scenario, &entity).await;
        assert_eq!(
            horse(&entity).get_owner(),
            Some(owner.player.gameprofile.id),
            "mutable event payload redirected original owner"
        );
        assert!(tamed_advancement(&owner));
        assert!(!tamed_advancement(&other));
        scenario.stop().await;
    })
    .await;
}

struct CancelExit;
impl EventHandler<EntityDismountEvent> for CancelExit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityDismountEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}
impl EventHandler<VehicleExitEvent> for CancelExit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut VehicleExitEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test]
async fn feat_horses_s2_cancelled_dismount_and_vehicle_exit_keep_rider_links() {
    for vehicle_event in [false, true] {
        run(async {
            let scenario = Scenario::new().await;
            let owner = scenario.join(Uuid::new_v4(), "HorseExit").await;
            let entity = spawn(&owner.player.world());
            interact(&scenario, &owner, &entity);
            if vehicle_event {
                scenario
                    .server
                    .plugin_manager
                    .register::<VehicleExitEvent, _>(
                        Arc::new(CancelExit),
                        EventPriority::Normal,
                        true,
                    );
            } else {
                scenario
                    .server
                    .plugin_manager
                    .register::<EntityDismountEvent, _>(
                        Arc::new(CancelExit),
                        EventPriority::Normal,
                        true,
                    );
            }
            owner.packets();
            script(&entity, 0, 0);
            opportunity(&scenario, &entity).await;
            assert_mounted(&owner, &entity);
            assert_eq!(horse(&entity).get_temper(), 5);
            assert!(!horse(&entity).has_flag(FLAG_STANDING));
            assert!(
                statuses(&owner, &entity).is_empty(),
                "cancelled exit announced a buck"
            );
            scenario.stop().await;
        })
        .await;
    }
}
