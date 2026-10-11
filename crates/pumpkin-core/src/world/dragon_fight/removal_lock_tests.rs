use super::{DragonFight, DragonRespawnStage, EntityBase, respawn_tests::Fixture};
use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;
use std::{
    cell::RefCell,
    sync::{Mutex, TryLockError, mpsc},
    thread,
};
use uuid::Uuid;

#[derive(Debug, PartialEq)]
enum RemovalObservation {
    Waiting,
    Marked,
}

type RemovalObserver = Box<dyn Fn(RemovalObservation)>;

thread_local! {
    static REMOVAL_OBSERVER: RefCell<Option<RemovalObserver>> = RefCell::new(None);
    static COMPLETION_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn observe_waiting(notifications: &Mutex<Vec<Uuid>>) {
    // Probe while completion owns the queue, before the real removal lock blocks.
    if matches!(notifications.try_lock(), Err(TryLockError::WouldBlock)) {
        REMOVAL_OBSERVER.with_borrow(|observer| {
            if let Some(observer) = observer {
                observer(RemovalObservation::Waiting);
            }
        });
    }
}

pub(super) fn observe_marked() {
    REMOVAL_OBSERVER.with_borrow(|observer| {
        if let Some(observer) = observer {
            observer(RemovalObservation::Marked);
        }
    });
}

pub(super) fn observe_completion() {
    if let Some(observer) = COMPLETION_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respawn_completion_serializes_crystal_removal_state() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    let crystal = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let destroying = crystal.clone();
    let runtime = tokio::runtime::Handle::current();
    let (worker_sender, worker_receiver) = mpsc::channel();
    COMPLETION_OBSERVER.with_borrow_mut(|observer| {
        *observer = Some(Box::new(move || {
            // Drive the real final tick; stop at its last check with both guards held.
            let (sender, receiver) = mpsc::channel();
            let worker = thread::spawn(move || {
                let _runtime = runtime.enter();
                REMOVAL_OBSERVER.with_borrow_mut(|observer| {
                    *observer = Some(Box::new(move |observation| {
                        sender.send(observation).unwrap();
                    }));
                });
                assert!(destroying.damage_with_context(
                    destroying.as_ref(),
                    1.0,
                    DamageType::EXPLOSION,
                    None,
                    None,
                    None,
                ));
            });
            // Without serialization the first signal is Marked, so this fails without a timeout.
            let first = receiver.recv().unwrap();
            worker_sender.send((worker, receiver)).unwrap();
            assert_eq!(first, RemovalObservation::Waiting);
        }));
    });
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    let (worker, observations) = worker_receiver.recv().unwrap();
    assert_eq!(observations.recv().unwrap(), RemovalObservation::Marked);
    worker.join().unwrap();
    assert!(crystal.get_entity().is_removed());
    {
        let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        assert!(fight.respawn_stage.is_none());
        assert!(!fight.dragon_killed && fight.dragon_uuid.is_some());
    };
    fixture.world.level.shutdown().await.unwrap();
}
