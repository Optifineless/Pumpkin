use super::*;
use pumpkin_data::{entity::EntityType, game_event::GameEvent};
use pumpkin_util::math::vector2::Vector2;
use std::sync::atomic::Ordering::Relaxed;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_existing_allay_bee_phantom_flap_ticks() {
    for (kind, ticks, expected) in [
        (&EntityType::ALLAY, 3, &[1, 2, 3][..]),
        (&EntityType::BEE, 4, &[0, 1, 1, 2][..]),
        (&EntityType::PHANTOM, 25, &[][..]),
        (&EntityType::VEX, 4, &[0, 0, 0, 0][..]),
    ] {
        let s = scenario(true, true);
        s.world
            .forced_chunks
            .lock()
            .unwrap()
            .insert(Vector2::new(0, 0));
        let actor = crate::entity::r#type::from_type(
            kind,
            Vector3::new(8.5, 95.0, 8.5),
            &s.world,
            uuid::Uuid::new_v4(),
        );
        let mob = actor.get_mob().unwrap().get_mob_entity();
        mob.goals_selector.lock().unwrap().clear();
        mob.target_selector.lock().unwrap().clear();
        actor
            .get_entity()
            .velocity
            .store(Vector3::new(0.1, 0.0, 0.0));
        s.world.add_entity_silent(actor.clone());
        let mut total = 0;
        for tick in 1..=ticks {
            let before = actor.get_entity().pos.load();
            s.world.tick(&s.server);
            let entity = actor.get_entity();
            assert_eq!(mob.ticks_lived.load(Relaxed), tick, "real species clock");
            assert_ne!(entity.pos.load(), before, "normal species travel must move");
            assert!(!entity.on_ground.load(Relaxed));
            let sink = s.sink.lock().unwrap();
            let events: Vec<_> = sink
                .forwarded
                .iter()
                .filter(|e| {
                    e.source
                        .as_ref()
                        .is_some_and(|source| source.id == entity.entity_id)
                })
                .collect();
            total = events.len();
            if let Some(expected) = expected.get(tick as usize - 1) {
                assert_eq!(total, *expected, "{} tick {tick}", kind.resource_name);
            } else {
                assert!(total <= 1, "one Phantom flap per complete 25-tick period");
            }
            assert!(
                events
                    .iter()
                    .all(|e| e.event == GameEvent::Flap && e.affected_state.is_none())
            );
        }
        assert_eq!(total, expected.last().copied().unwrap_or(1));
    }
    outside_height_flapping();
}

fn outside_height_flapping() {
    for above in [true, false] {
        let s = scenario(true, true);
        s.world
            .forced_chunks
            .lock()
            .unwrap()
            .insert(Vector2::new(0, 0));
        let y = if above {
            s.world.get_top_y() + 2
        } else {
            s.world.get_bottom_y() - 2
        };
        let actor = crate::entity::r#type::from_type(
            &EntityType::ALLAY,
            Vector3::new(8.5, f64::from(y), 8.5),
            &s.world,
            uuid::Uuid::new_v4(),
        );
        let mob = actor.get_mob().unwrap().get_mob_entity();
        mob.goals_selector.lock().unwrap().clear();
        mob.target_selector.lock().unwrap().clear();
        actor
            .get_entity()
            .velocity
            .store(Vector3::new(0.1, 0.0, 0.0));
        s.world.add_entity_silent(actor.clone());
        let start = actor.get_entity().pos.load();
        s.world.tick(&s.server);
        assert_ne!(
            actor.get_entity().pos.load(),
            start,
            "real out-of-height travel"
        );
        let sink = s.sink.lock().unwrap();
        assert_eq!(
            sink.forwarded.len(),
            1,
            "resident out-of-height air permits real FLAP"
        );
        assert_eq!(sink.forwarded[0].event, GameEvent::Flap);
        assert_eq!(sink.forwarded[0].affected_state, None);
        assert_eq!(
            sink.forwarded[0].source.as_ref().unwrap().uuid,
            actor.get_entity().entity_uuid
        );
        drop(sink);
        s.world.level.loaded_chunks.remove(&Vector2::new(0, 0));
        let start = actor.get_entity().pos.load();
        actor
            .get_entity()
            .move_entity(actor.as_ref(), Vector3::new(0.25, 0.0, 0.0));
        assert_eq!(
            actor.get_entity().pos.load(),
            start + Vector3::new(0.25, 0.0, 0.0)
        );
        assert_eq!(
            s.sink.lock().unwrap().forwarded.len(),
            1,
            "out-of-height air still requires resident XZ terrain"
        );
        assert!(!s.world.level.is_chunk_loaded(&Vector2::new(0, 0)));
    }
}
