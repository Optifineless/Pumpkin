use super::{MapData, MapManager, storage::read_map};
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
        // SavedDataStorage.get is synchronous in vanilla; Pumpkin's tick must never do disk I/O.
        // A miss is retried by the next map tick after this worker populates the cache.
        self.tasks
            .spawn_blocking(move || match load_record(&world, id) {
                Ok(Some(map)) => {
                    maps.entry(id).or_insert_with(|| Arc::new(Mutex::new(map)));
                }
                Ok(None) => {}
                Err(error) => tracing::error!("Failed to load map {id}: {error}"),
            });
    }

    /// Loads a cache miss off the tick thread and returns the cached record.
    pub async fn load_map(&self, id: i32) -> Option<Arc<Mutex<MapData>>> {
        if let Some(map) = self.maps.get(&id) {
            return Some(map.clone());
        }
        let world = self.storage_path.clone()?;
        let maps = self.maps.clone();
        self.tasks
            .spawn_blocking(move || {
                let map = load_record(&world, id).ok()??;
                Some(
                    maps.entry(id)
                        .or_insert_with(|| Arc::new(Mutex::new(map)))
                        .clone(),
                )
            })
            .await
            .ok()?
    }

    /// Drains pending reads before the final saved-data write.
    pub async fn drain(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

fn load_record(world: &std::path::Path, id: i32) -> std::io::Result<Option<MapData>> {
    for (path, legacy) in [
        (world.join(format!("data/minecraft/maps/{id}.dat")), false),
        (world.join(format!("data/minecraft/map_{id}.dat")), true),
        (world.join(format!("data/map_{id}.dat")), true),
    ] {
        match read_map(&path) {
            Ok(mut map) => {
                if legacy {
                    map.saved_tag = None;
                }
                return Ok(Some(map));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}
