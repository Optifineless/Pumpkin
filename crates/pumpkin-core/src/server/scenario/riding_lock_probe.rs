//! Test-only deadlock tripwire for watched mounts, before dismount helpers take their mutex.
use crate::entity::{Entity, EntityBase};
use std::{
    collections::HashSet,
    sync::{Arc, LazyLock, Mutex},
};
use uuid::Uuid;
static WATCHED: LazyLock<Mutex<HashSet<Uuid>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
pub struct Watch(Uuid);
pub fn watch(entity: &Arc<dyn EntityBase>) -> Watch {
    let uuid = entity.get_entity().entity_uuid;
    WATCHED.lock().unwrap().insert(uuid);
    Watch(uuid)
}
impl Drop for Watch {
    fn drop(&mut self) {
        WATCHED.lock().unwrap().remove(&self.0);
    }
}
/// Inspect one registration without depending on unrelated scenarios' watches.
pub(super) fn is_watched(uuid: Uuid) -> bool {
    WATCHED.lock().unwrap().contains(&uuid)
}
/// Fail before a watched mount's synchronous dismount could re-lock its passenger mutex.
pub fn assert_unlocked(entity: &Entity) {
    let watched = WATCHED.lock().unwrap().contains(&entity.entity_uuid);
    if watched {
        assert!(
            entity.passengers.try_lock().is_ok(),
            "audit row 1: dismount entered with passengers locked (would deadlock)"
        );
    }
}
