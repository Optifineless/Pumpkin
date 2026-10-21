//! Bounded real-entry probes for late insertion rejection and a forced `ArcSwap` retry.
use crate::{
    entity::{EntityBase, player::Player},
    world::World,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
};

use std::cell::RefCell;

thread_local! {
    // The synchronous real-entry worker installs and reaches its own probes.
    static PROBES: RefCell<Vec<(usize, Arc<Probe>)>> = const { RefCell::new(Vec::new()) };
    static WORLD_SNAPSHOTS: RefCell<Vec<(i32, WorldSnapshotTransition)>> = const { RefCell::new(Vec::new()) };
}

type WorldSnapshotTransition = Box<dyn FnOnce(&Arc<World>, &Player)>;

pub struct WorldSnapshotRegistration(i32);

impl Drop for WorldSnapshotRegistration {
    fn drop(&mut self) {
        WORLD_SNAPSHOTS.with(|snapshots| {
            snapshots
                .borrow_mut()
                .retain(|(player, _)| *player != self.0);
        });
    }
}

pub fn install_world_snapshot(
    player: &Player,
    transition: impl FnOnce(&Arc<World>, &Player) + 'static,
) -> WorldSnapshotRegistration {
    let id = player.entity_id();
    WORLD_SNAPSHOTS.with(|snapshots| snapshots.borrow_mut().push((id, Box::new(transition))));
    WorldSnapshotRegistration(id)
}

pub fn after_world_snapshot(world: &Arc<World>, player: &Player) {
    let transition = WORLD_SNAPSHOTS.with(|snapshots| {
        let mut snapshots = snapshots.borrow_mut();
        let index = snapshots
            .iter()
            .position(|(id, _)| *id == player.entity_id())?;
        Some(snapshots.remove(index).1)
    });
    if let Some(transition) = transition {
        transition(world, player);
    }
}

pub fn reservations_released(world: &World) -> bool {
    world.spawn_uuids.try_lock().is_ok_and(|ids| ids.is_empty())
}

pub struct Probe {
    rejections: AtomicUsize,
    retry: AtomicBool,
    unrelated: Option<Arc<dyn EntityBase>>,
    pub(crate) attempts: AtomicUsize,
}

pub struct Registration(usize);

impl Drop for Registration {
    fn drop(&mut self) {
        PROBES.with(|probes| probes.borrow_mut().retain(|(world, _)| *world != self.0));
    }
}

pub fn install(
    world: &Arc<World>,
    rejections: usize,
    unrelated: Option<Arc<dyn EntityBase>>,
) -> (Registration, Arc<Probe>) {
    let key = Arc::as_ptr(world) as usize;
    let probe = Arc::new(Probe {
        rejections: AtomicUsize::new(rejections),
        retry: AtomicBool::new(unrelated.is_some()),
        unrelated,
        attempts: AtomicUsize::new(0),
    });
    PROBES.with(|probes| probes.borrow_mut().push((key, probe.clone())));
    (Registration(key), probe)
}

pub(super) fn probe(world: &World) -> Option<Arc<Probe>> {
    let key = std::ptr::from_ref(world) as usize;
    PROBES.with(|probes| {
        probes
            .borrow()
            .iter()
            .find(|(id, _)| *id == key)
            .map(|(_, probe)| probe.clone())
    })
}

impl Probe {
    pub(super) fn reject(&self) -> bool {
        self.rejections
            .fetch_update(SeqCst, SeqCst, |left| left.checked_sub(1))
            .is_ok()
    }

    pub(super) fn attempt(&self, world: &World) {
        self.attempts.fetch_add(1, SeqCst);
        if self.retry.swap(false, SeqCst)
            && let Some(unrelated) = &self.unrelated
        {
            world.entities.rcu(|current| {
                let mut next = (**current).clone();
                next.push(unrelated.clone());
                next
            });
        }
    }
}
