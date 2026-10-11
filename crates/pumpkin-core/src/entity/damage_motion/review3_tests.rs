#![expect(
    clippy::unwrap_used,
    reason = "Motion regression fixtures and channels must be valid"
)]
use super::*;
use crate::{
    entity::{
        EntityBase,
        living::damage_transaction::test_hooks::{self, Point},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::SeqCst},
        mpsc,
    },
    time::Duration,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_mace_braking_waits_for_attacker_motion_restoration() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let attacker = TestPlayer::new(&world).player;
    let victim = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![attacker.clone(), victim.clone()]));
    attacker
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::MACE));
    attacker.last_attacked_ticks.store(100, Relaxed);
    attacker.living_entity.fall_distance.store(2.0);
    let old = Vector3::new(0.8, -1.0, 0.2);
    let temporary = Vector3::new(4.0, 2.0, 5.0);
    let serialized = Arc::new(AtomicBool::new(false));
    let (send, receive) = mpsc::channel();
    std::thread::scope(|scope| {
        let target = attacker.clone();
        let reached = serialized.clone();
        scope.spawn(move || {
            let _motion_owner = target.living_entity.own_damage();
            target.get_entity().velocity.store(temporary);
            send.send(()).unwrap();
            reached.store(target.living_entity.wait_until_damage_contended(), SeqCst);
            target.get_entity().velocity.store(old);
        });
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
        attacker.attack(&(victim.clone() as Arc<dyn EntityBase>));
    });
    assert!(
        serialized.load(SeqCst),
        "mace braking bypassed the attacker's motion owner"
    );
    assert_eq!(
        attacker.get_entity().velocity.load(),
        Vector3::new(old.x, f64::from(0.01f32), old.z)
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_final_player_tracking_delivers_motion_with_one_acquisition() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = fixture.player;
    let before = player.living_entity.damage_entry_count();
    let observed = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let record = observed.clone();
    let target = player.clone();
    test_hooks::install(move |point| {
        if point == Point::PlayerMotionFlush {
            record.store(target.living_entity.damage_entry_count() - before, SeqCst);
        }
    });
    player.living_entity.tick(player.as_ref(), &server);
    assert_eq!(observed.load(SeqCst), usize::MAX);
    let before_tracking = player.living_entity.damage_entry_count();
    player.living_entity.flush_tracked_player_motion();
    // Final tracking acquires ownership once; player ticking does not deliver motion.
    assert_eq!(observed.load(SeqCst), before_tracking - before + 1);
    crate::server::fixture_lifecycle::finish().await;
}
