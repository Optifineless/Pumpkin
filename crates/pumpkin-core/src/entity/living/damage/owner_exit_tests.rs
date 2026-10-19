use super::*;
use crate::{
    entity::living::damage_transaction::test_hooks::{self, Point},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use std::{cell::Cell, sync::mpsc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_pending_hurt_retires_before_contended_exit() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    victim.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    victim.get_entity().set_has_no_gravity(true);
    let (start, started) = mpsc::channel();
    let (finish, finished) = mpsc::channel();
    let contender = victim.clone();
    let task_server = server;
    std::thread::spawn(move || {
        started.recv_timeout(Duration::from_secs(10)).unwrap();
        // Real living ticks must resume after the previous serial hurt has exited.
        for _ in 0..11 {
            contender
                .living_entity
                .tick(contender.as_ref(), &task_server);
        }
        let admitted = contender.damage(contender.as_ref(), 2.0, DamageType::GENERIC);
        finish.send(admitted).unwrap();
    });
    let exiting = victim.clone();
    let armed = Cell::new(true);
    test_hooks::install(move |point| match point {
        Point::OwnerReleasing if armed.get() => {
            start.send(()).unwrap();
            assert!(exiting.living_entity.wait_until_damage_contended());
        }
        Point::OwnerReleased if armed.replace(false) => {
            assert!(finished.recv_timeout(Duration::from_secs(10)).unwrap());
        }
        _ => {}
    });

    assert!(victim.damage(victim.as_ref(), 4.0, DamageType::GENERIC));
    test_hooks::install(|_| {});
    assert_eq!(victim.living_entity.health.load(), 14.0);
    assert_eq!(victim.living_entity.last_damage_taken.load(), 2.0);
}
