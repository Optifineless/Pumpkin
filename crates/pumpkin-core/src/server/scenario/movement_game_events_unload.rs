use super::*;
use pumpkin_data::game_event::GameEvent;

fn steps(sink: &crate::world::game_events::test_support::Observations, id: Uuid) -> usize {
    sink.forwarded
        .iter()
        .filter(|event| {
            event.event == GameEvent::Step
                && event
                    .source
                    .as_ref()
                    .is_some_and(|source| source.uuid == id)
        })
        .count()
}

async fn activated(s: &Scenario, world: &World, id: Uuid) -> Arc<dyn EntityBase> {
    s.until("movement entity activation publication", || {
        world.get_entity_by_uuid(id).is_some_and(|current| {
            world
                .entity_tracker
                .get_tracked_entity(current.get_entity().entity_id)
                .is_some_and(|tracked| {
                    std::ptr::eq(tracked.entity.get_entity(), current.get_entity())
                })
        })
    })
    .await;
    world.get_entity_by_uuid(id).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_entity_unload_reload_twice() {
    run(async {
        let s = Scenario::new().await;
        let p = s.join(Uuid::new_v4(), "MoveUnload").await;
        let world = p.player.world();
        let cow = Scenario::mob(&world, &EntityType::COW, Vector3::new(3.5, 64.0, 10.5));
        let id = cow.get_entity().entity_uuid;
        cow.get_mob().unwrap().get_mob_entity().set_no_ai(true);
        cow.get_living_entity().unwrap().health.store(7.0);
        s.prepare_area(&world, Vector2::new(10, 0)).await;
        for _ in 0..2 {
            let old = world.get_entity_by_uuid(id).unwrap();
            let sink = crate::world::game_events::test_support::attach(&world);
            old.get_entity()
                .move_entity(old.as_ref(), Vector3::new(1.5, -0.08, 0.0));
            assert_eq!(steps(&sink.lock().unwrap(), id), 0);
            let position = old.get_entity().pos.load();
            s.command("tp MoveUnload 163.5 64 3.5");
            p.loaded(&s.server);
            s.until("movement entity unload tasks", || {
                s.server.tasks.is_empty() && p.is_idle()
            })
            .await;
            s.ticks(1).await;
            assert!(
                old.get_entity().is_removed(),
                "old entity must actually retire"
            );
            assert!(world.get_entity_by_uuid(id).is_none());
            old.get_entity()
                .move_entity(old.as_ref(), Vector3::new(2.0, 0.0, 0.0));
            assert_eq!(
                steps(&sink.lock().unwrap(), id),
                0,
                "retired Arc cannot emit"
            );
            s.command("tp MoveUnload 3.5 64 3.5");
            p.loaded(&s.server);
            s.until("movement entity reload tasks", || {
                s.server.tasks.is_empty() && p.is_idle()
            })
            .await;
            s.ticks(1).await;
            let current = activated(&s, &world, id).await;
            assert!(!std::ptr::eq(old.get_entity(), current.get_entity()));
            assert_eq!(current.get_entity().pos.load(), position);
            assert_eq!(current.get_living_entity().unwrap().health.load(), 7.0);
            assert!(current.get_mob().unwrap().get_mob_entity().is_no_ai());
            current
                .get_entity()
                .move_entity(current.as_ref(), Vector3::new(0.25, -0.08, 0.0));
            assert_eq!(
                current.get_entity().pos.load(),
                position + Vector3::new(0.25, 0.0, 0.0)
            );
            assert_eq!(
                steps(&sink.lock().unwrap(), id),
                0,
                "reload discards transient distance"
            );
            current
                .get_entity()
                .move_entity(current.as_ref(), Vector3::new(1.5, -0.08, 0.0));
            assert_eq!(
                current.get_entity().pos.load(),
                position + Vector3::new(1.75, 0.0, 0.0)
            );
            assert_eq!(
                steps(&sink.lock().unwrap(), id),
                1,
                "new instance independently crosses threshold"
            );
        }
        s.stop().await;
    })
    .await;
}
