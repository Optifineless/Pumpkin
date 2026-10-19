use super::{
    MapData, MapManager,
    frame_cleanup::{apply_pending, discard_missing},
    storage::read_map,
};
use std::sync::{Arc, Mutex, atomic::Ordering};

impl MapManager {
    /// Reconciles Pumpkin's next-ID counter and IDs already present in storage.
    pub fn reconcile_counter(&self, next: i32) {
        self.next_id
            .fetch_max(u64::from(next as u32), Ordering::SeqCst);
    }

    pub(super) fn reserve_map_id(&self, id: i32) {
        self.known_ids.insert(id);
        // MapIndex.getNextMapId wraps Java int IDs; retain a wider next counter at MAX_VALUE.
        self.next_id
            .fetch_max(u64::from(id as u32) + 1, Ordering::SeqCst);
    }

    /// Reserves an unused ID, like `ServerLevel.getFreeMapId` / `MapIndex.getNextMapId`.
    pub fn next_map_id(&self) -> i32 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst) as i32;
            if !self.maps.contains_key(&id) && self.known_ids.insert(id) {
                return id;
            }
        }
    }

    pub(crate) fn next_counter(&self) -> i32 {
        self.next_id.load(Ordering::SeqCst) as i32
    }

    pub(super) fn request_load(&self, id: i32) {
        let Some(world) = self.storage_path.clone() else {
            return;
        };
        if !self.loading.insert(id) {
            return;
        }
        let maps = self.maps.clone();
        let cleanup = self.frame_cleanup.clone();
        let loading = self.loading.clone();
        // SavedDataStorage.get is synchronous in vanilla; Pumpkin's tick must never do disk I/O.
        // A miss is retried by the next map tick after this worker populates the cache.
        #[cfg(test)]
        let gate = self
            .load_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.tasks.spawn_blocking(move || {
            #[cfg(test)]
            if let Some((entered, release)) = gate {
                let _ = entered.send(());
                assert!(
                    release
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .is_ok(),
                    "bounded loader gate expired"
                );
            }
            match load_record(&world, id) {
                Ok(Some(map)) => {
                    let map = maps
                        .entry(id)
                        .or_insert_with(|| Arc::new(Mutex::new(map)))
                        .clone();
                    apply_pending(&cleanup, id, &map);
                }
                Ok(None) => discard_missing(&cleanup, id),
                Err(error) => tracing::error!("Failed to load map {id}: {error}"),
            }
            loading.remove(&id);
        });
    }

    /// Loads a cache miss off the tick thread and returns the cached record.
    pub async fn load_map(&self, id: i32) -> Option<Arc<Mutex<MapData>>> {
        if let Some(map) = self.maps.get(&id) {
            return Some(map.clone());
        }
        let world = self.storage_path.clone()?;
        let maps = self.maps.clone();
        let cleanup = self.frame_cleanup.clone();
        self.tasks
            .spawn_blocking(move || {
                let map = load_record(&world, id).ok()??;
                let map = maps
                    .entry(id)
                    .or_insert_with(|| Arc::new(Mutex::new(map)))
                    .clone();
                apply_pending(&cleanup, id, &map);
                Some(map)
            })
            .await
            .ok()?
    }

    #[cfg(test)]
    pub(crate) fn pause_next_load_for_test(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, receiver) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        *self
            .load_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((entered, gate));
        (receiver, release)
    }

    /// Drains pending reads before the final saved-data write.
    pub async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
        // One bounded retry also observes a cold cleanup whose first read failed.
        self.retry_frame_cleanup();
        self.tasks.wait().await;
        self.report_pending_frame_cleanup();
    }
}

fn load_record(world: &std::path::Path, id: i32) -> std::io::Result<Option<MapData>> {
    for path in [
        world.join(format!("data/minecraft/maps/{id}.dat")),
        world.join(format!("data/minecraft/map_{id}.dat")),
        world.join(format!("data/map_{id}.dat")),
    ] {
        match read_map(&path) {
            Ok(map) => return Ok(Some(map)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}
