use super::support::*;
use crate::entity::ai::goal::{
    Goal, bow_attack::BowAttackGoal, ranged_crossbow_attack::RangedCrossbowAttackGoal,
};
use pumpkin_data::{
    data_component_impl::{EquipmentSlot, UseEffectsImpl},
    entity::EntityType,
    item::Item,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_fishing_cast_veto_preserves_newer_hook() {
    run(|f| {
        for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
            for newer in [false, true] {
                if newer && veto == Veto::None {
                    continue;
                }
                let mut p = player(f);
                let hooks = Hooks::install(&f.server, &p.player);
                let mut rod = stack(1, &Item::FISHING_ROD);
                rod.set_data_component(UseEffectsImpl {
                    interact_vibrations: true,
                    ..UseEffectsImpl::DEFAULT
                });
                p.player.inventory().set_stack(0, rod.clone());
                let weak = Arc::downgrade(&hooks);
                let edit: Option<Edit> = newer.then(|| {
                    Arc::new(move |p: &Arc<crate::entity::player::Player>| {
                        let hooks = weak.upgrade().unwrap();
                        hooks.state.lock().unwrap().partial = true;
                        let crate::net::ClientPlatform::Java(client) = p.client.as_ref() else {
                            panic!("Java cast fixture required")
                        };
                        client.handle_use_item(
                            p,
                            &pumpkin_protocol::java::server::play::SUseItem {
                                hand: pumpkin_protocol::VarInt(0),
                                sequence: pumpkin_protocol::VarInt(5),
                                yaw: 0.0,
                                pitch: 0.0,
                            },
                            &p.world().server.upgrade().unwrap(),
                        );
                    }) as Edit
                });
                hooks.configure(veto, false, edit);
                p.take_packets();
                use_item(&p, f, Hand::Right);
                let success = veto == Veto::None || newer;
                let id = p.player.fishing_bobber.load(SeqCst);
                if success {
                    assert!(f.world().get_entity_by_id(id).is_some());
                } else {
                    assert_eq!(id, -1);
                }
                if newer && veto != Veto::None {
                    assert_ne!(
                        id,
                        hooks.state.lock().unwrap().order[0].1,
                        "rejected old hook replaced the newer hook"
                    );
                }
                assert!(p.player.inventory().get_stack(0).are_equal(&rod));
                assert_eq!(used(&p.player, &Item::FISHING_ROD), i32::from(success));
                assert_eq!(
                    hooks.state.lock().unwrap().casts,
                    if newer && veto != Veto::None { 2 } else { 1 }
                );
                assert_eq!(hooks.state.lock().unwrap().vibrations, usize::from(success));
                let packets = p.take_packets();
                if !success {
                    assert_quiet(&packets);
                }
                if success {
                    let hook = f.world().get_entity_by_id(id).unwrap();
                    hook.get_entity().on_ground.store(true, SeqCst);
                    hooks.configure(Veto::None, false, None);
                    use_item(&p, f, Hand::Right);
                    assert_eq!(p.player.fishing_bobber.load(SeqCst), -1);
                    assert_eq!(p.player.inventory().get_stack(0).get_damage(), 2);
                }
            }
        }
        cast_event_replacement(f);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_ranged_ai_launch_once() {
    run(|f| {
        let p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        for crossbow in [false, true] {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                let mob = f.mob(if crossbow {
                    &EntityType::PILLAGER
                } else {
                    &EntityType::SKELETON
                });
                mob.get_entity().set_pos(Vector3::new(7.5, 64.0, 8.5));
                let ai = mob.get_mob().unwrap();
                ai.get_mob_entity().set_target(Some(p.player.clone()));
                ai.get_mob_entity()
                    .living_entity
                    .entity_equipment
                    .lock()
                    .unwrap()
                    .put(
                        &EquipmentSlot::MAIN_HAND,
                        stack(
                            1,
                            if crossbow {
                                &Item::CROSSBOW
                            } else {
                                &Item::BOW
                            },
                        ),
                    );
                let callback = mob.clone();
                hooks.configure(veto, false, None);
                {
                    let mut s = hooks.state.lock().unwrap();
                    s.observe_any = true;
                    s.observer = Some(Arc::new(move |_| {
                        let mob = callback.clone();
                        let (send, recv) = mpsc::sync_channel(1);
                        std::thread::spawn(move || {
                            let available = mob
                                .get_living_entity()
                                .unwrap()
                                .entity_equipment
                                .try_lock()
                                .is_ok();
                            let _ = send.send(available);
                        });
                        assert!(
                            recv.recv_timeout(Duration::from_secs(2)).unwrap(),
                            "ranged AI retained equipment across callback"
                        );
                    }));
                };
                let before = f.world().entities.load().len();
                let mut goal: Box<dyn Goal> = if crossbow {
                    Box::new(RangedCrossbowAttackGoal::new(1.0, 15.0))
                } else {
                    Box::new(BowAttackGoal::new(1.0, 20, 15.0))
                };
                goal.start(ai);
                for _ in 0..120 {
                    goal.tick(ai);
                    if hooks.counts().0 > 0 {
                        break;
                    }
                }
                assert_eq!(
                    hooks.counts().0,
                    1,
                    "goal lost or duplicated its launch hook"
                );
                assert_eq!(hooks.counts().1, usize::from(veto != Veto::Launch));
                assert_eq!(
                    f.world().entities.load().len() - before,
                    usize::from(veto == Veto::None)
                );
                goal.stop(ai);
            }
        }
    })
    .await;
}

fn cast_event_replacement(f: &crate::entity::death_test_world::DeathTestWorld) {
    // A newer cast installed by PlayerFishEvent also owns the pending ID.
    let p = player(f);
    let hooks = Hooks::install(&f.server, &p.player);
    p.player
        .inventory()
        .set_stack(0, stack(1, &Item::FISHING_ROD));
    let weak = Arc::downgrade(&hooks);
    hooks.state.lock().unwrap().cast_edit = Some(Arc::new(move |p| {
        weak.upgrade().unwrap().state.lock().unwrap().veto = Some(Veto::None);
        let crate::net::ClientPlatform::Java(client) = p.client.as_ref() else {
            panic!("Java cast fixture required")
        };
        client.handle_use_item(
            p,
            &pumpkin_protocol::java::server::play::SUseItem {
                hand: pumpkin_protocol::VarInt(0),
                sequence: pumpkin_protocol::VarInt(8),
                yaw: 0.0,
                pitch: 0.0,
            },
            &p.world().server.upgrade().unwrap(),
        );
    }));
    use_item(&p, f, Hand::Right);
    assert_eq!(
        hooks.counts().0,
        1,
        "older cast overwrote a newer cast-event hook"
    );
    assert_eq!(used(&p.player, &Item::FISHING_ROD), 1);
    assert!(
        p.player
            .world()
            .get_entity_by_id(p.player.fishing_bobber.load(SeqCst))
            .is_some()
    );
}
