use super::*;
use crate::entity::{EntityBase, mob::bat::BatEntity};
use pumpkin_data::{entity::EntityType, game_event::GameEvent};
use pumpkin_protocol::java::server::play::SMoveVehicle;
use pumpkin_util::math::vector2::Vector2;
use std::{sync::Arc, sync::atomic::Ordering::Relaxed};

fn mob(s: &Scenario, kind: &'static EntityType, pos: Vector3<f64>) -> Arc<dyn EntityBase> {
    let entity = crate::entity::r#type::from_type(kind, pos, &s.world, uuid::Uuid::new_v4());
    s.world.add_entity_silent(entity.clone());
    entity
}

fn source_events(s: &Scenario, id: i32, event: GameEvent) -> usize {
    s.sink
        .lock()
        .unwrap()
        .forwarded
        .iter()
        .filter(|e| e.event == event && e.source.as_ref().is_some_and(|source| source.id == id))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_nonplayer_move_and_flap() {
    let s = scenario(true, true);
    s.world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    for x in 0..16 {
        for z in 0..16 {
            place(
                &s,
                &BlockPos::new(x, 64, z),
                if x < 6 {
                    Block::DIRT_PATH.default_state.id
                } else {
                    Block::STONE.default_state.id
                },
            );
        }
    }
    let cow = mob(&s, &EntityType::COW, Vector3::new(4.5, 64.9375, 8.5));
    let inner = cow.get_mob().unwrap().get_mob_entity();
    inner.goals_selector.lock().unwrap().clear();
    inner.target_selector.lock().unwrap().clear();
    cow.get_entity().on_ground.store(true, Relaxed);
    assert!(inner.navigator.lock().unwrap().move_to_coords(
        10.5,
        65.0,
        8.5,
        1.0,
        &inner.living_entity
    ));
    let health = inner.living_entity.health.load();
    let mut rose = false;
    let mut distance = 0.0;
    for _ in 0..100 {
        let before = cow.get_entity().pos.load();
        s.world.tick(&s.server);
        let after = cow.get_entity().pos.load();
        rose |= after.y > before.y + 1e-7;
        distance += (after.x - before.x).hypot(after.z - before.z);
        if after.x > 6.5 {
            break;
        }
    }
    assert!(rose, "normal navigation must step up the path lip");
    assert!(
        cow.get_entity().pos.load().x > 6.5,
        "normal AI ticks must travel"
    );
    assert!(
        distance > 1.66 && distance < 3.2,
        "bounded actual path distance: {distance}"
    );
    assert_eq!(
        inner.living_entity.health.load(),
        health,
        "physics/health control"
    );
    assert!(
        !inner.living_entity.jumping.load(Relaxed),
        "step-up is not a jump"
    );
    assert_eq!(
        source_events(&s, cow.get_entity().entity_id, GameEvent::Step),
        1,
        "actual clipped travel emits once"
    );
    cow.get_mob().unwrap().get_mob_entity().set_no_ai(true);

    flap_and_silent_controls(&s, &cow);
}

fn flap_and_silent_controls(s: &Scenario, cow: &Arc<dyn EntityBase>) {
    let bat = mob(s, &EntityType::BAT, Vector3::new(8.5, 74.0, 8.5));
    bat.cast_any()
        .downcast_ref::<BatEntity>()
        .unwrap()
        .set_roosting(false);
    for _ in 0..9 {
        s.world.tick(&s.server);
    }
    assert_eq!(
        source_events(s, bat.get_entity().entity_id, GameEvent::Flap),
        0,
        "not every aerial tick flaps"
    );
    s.world.tick(&s.server);
    assert_eq!(
        source_events(s, bat.get_entity().entity_id, GameEvent::Flap),
        1,
        "real Bat tick ten flaps once"
    );
    let sink = s.sink.lock().unwrap();
    let flap = sink
        .forwarded
        .iter()
        .find(|e| e.event == GameEvent::Flap)
        .unwrap();
    assert_eq!(flap.affected_state, None);
    drop(sink);

    let item = mob(s, &EntityType::TNT, Vector3::new(5.5, 65.0, 5.5));
    item.get_entity()
        .move_entity(item.as_ref(), Vector3::new(2.0, -0.08, 0.0));
    assert_eq!(
        source_events(s, item.get_entity().entity_id, GameEvent::Step),
        0,
        "PrimedTnt NONE policy"
    );
    let before = source_events(s, cow.get_entity().entity_id, GameEvent::Step);
    cow.get_entity().no_physics.store(true, Relaxed);
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::new(2.0, 0.0, 0.0));
    cow.get_entity().no_physics.store(false, Relaxed);
    cow.get_entity()
        .move_entity(cow.as_ref(), Vector3::default());
    assert_eq!(
        source_events(s, cow.get_entity().entity_id, GameEvent::Step),
        before,
        "no physics/zero displacement controls"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_vehicle_excludes_passenger_placement() {
    let s = scenario(true, true);
    let boat = mob(&s, &EntityType::OAK_BOAT, START);
    boat.get_entity().on_ground.store(true, Relaxed);
    boat.get_entity()
        .add_passenger(boat.clone(), s.owner.player.clone());
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
    assert_eq!(boat.get_entity().pos.load(), at(1.75));
    assert_eq!(s.owner.player.position(), at(1.75));
    assert_eq!(
        source_events(&s, boat.get_entity().entity_id, GameEvent::Step),
        1,
        "AbstractBoat EVENTS policy"
    );
    assert_eq!(
        source_events(&s, s.owner.player.entity_id(), GameEvent::Step),
        0,
        "rider placement stays silent"
    );
    position(&s, true, at(4.0), true);
    assert_eq!(counts(&s), (1, 1), "mounted movement packet is rejected");
}
