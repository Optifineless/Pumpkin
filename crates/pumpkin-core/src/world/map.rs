use crate::entity::player::Player;
use dashmap::DashMap;
use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use std::sync::{Arc, Mutex};

mod cache;
mod markers;
mod storage;
#[cfg(test)]
mod storage_tests;

// MapItemSavedData's Java constants.
pub(crate) const MAX_SCALE: i8 = 4;
pub(crate) const MAP_SIZE: i32 = 128;
pub(crate) const HALF_MAP_SIZE: i32 = MAP_SIZE / 2;

pub struct MapManager {
    pub maps: Arc<DashMap<i32, Arc<Mutex<MapData>>>>,
    save_gate: Arc<tokio::sync::Mutex<()>>,
    storage_path: Option<std::path::PathBuf>,
    next_id: std::sync::atomic::AtomicU64,
    known_ids: dashmap::DashSet<i32>,
    loading: Arc<dashmap::DashSet<i32>>,
    tasks: tokio_util::task::TaskTracker,
    #[cfg(test)]
    save_barrier: Mutex<Option<Arc<std::sync::Barrier>>>,
}

impl Default for MapManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MapManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            maps: Arc::new(DashMap::new()),
            save_gate: Arc::default(),
            storage_path: None,
            next_id: std::sync::atomic::AtomicU64::new(0),
            known_ids: dashmap::DashSet::new(),
            loading: Arc::default(),
            tasks: tokio_util::task::TaskTracker::new(),
            #[cfg(test)]
            save_barrier: Mutex::default(),
        }
    }

    #[must_use]
    /// Returns a cached map, queuing one worker read on a miss; retry on a later tick.
    pub fn get_map(&self, id: i32) -> Option<Arc<Mutex<MapData>>> {
        if let Some(map) = self.maps.get(&id) {
            return Some(map.clone());
        }
        self.request_load(id);
        None
    }

    #[must_use]
    pub fn create_map(
        &self,
        id: i32,
        dimension: Dimension,
        x: i32,
        z: i32,
        scale: i8,
    ) -> Arc<Mutex<MapData>> {
        let map = Arc::new(Mutex::new(MapData::new(dimension, x, z, scale)));
        self.reserve_map_id(id);
        // Never replace a cached record, even when a plugin supplies an already-used ID.
        self.maps.entry(id).or_insert(map).clone()
    }
}

pub struct MapData {
    pub scale: i8,
    pub locked: bool,
    pub dimension: Dimension,
    pub center_x: i32,
    pub center_z: i32,
    pub colors: Box<[u8; 128 * 128]>,
    pub decorations: Vec<MapDecoration>,
    pub dirty: bool,
    pub fully_updated: bool,
    pub tracking_position: bool,
    pub unlimited_tracking: bool,
    pub banners: Vec<pumpkin_nbt::tag::NbtTag>,
    pub frames: Vec<pumpkin_nbt::tag::NbtTag>,
    saved_tag: Option<pumpkin_nbt::compound::NbtCompound>,
}

impl MapData {
    #[must_use]
    pub fn new(dimension: Dimension, x: i32, z: i32, scale: i8) -> Self {
        Self {
            scale,
            locked: false,
            dimension,
            center_x: x,
            center_z: z,
            colors: Box::new([0; 128 * 128]),
            decorations: Vec::new(),
            dirty: true,
            fully_updated: false,
            tracking_position: true,
            unlimited_tracking: false,
            banners: Vec::new(),
            frames: Vec::new(),
            saved_tag: None,
        }
    }

    pub fn set_color(&mut self, x: usize, z: usize, color: u8) {
        if x < 128 && z < 128 {
            let idx = z * 128 + x;
            if self.colors[idx] != color {
                self.colors[idx] = color;
                self.dirty = true;
            }
        }
    }

    pub fn update(&mut self, player: &Player) {
        // MapItem.inventoryTick never updates pixels in a locked map.
        if self.locked {
            return;
        }
        let world = player.world();
        let scale = 1 << self.scale;
        let center_x = self.center_x;
        let center_z = self.center_z;

        let player_pos = player.position();
        let player_x = player_pos.x as i32;
        let player_z = player_pos.z as i32;

        let start_img_x = ((player_x - center_x) / scale + 64).clamp(0, 127) as usize;
        let start_img_z = ((player_z - center_z) / scale + 64).clamp(0, 127) as usize;

        let radius = 16;
        let (range_x, range_z) = if self.fully_updated {
            (
                (start_img_x.saturating_sub(radius))..(start_img_x + radius).min(128),
                (start_img_z.saturating_sub(radius))..(start_img_z + radius).min(128),
            )
        } else {
            self.fully_updated = true;
            (0..128, 0..128)
        };

        for img_x in range_x {
            let mut prev_y = -1;
            for img_z in range_z.clone() {
                let world_x = (img_x as i32 - 64) * scale + center_x;
                let world_z = (img_z as i32 - 64) * scale + center_z;

                let top_y = world.get_top_block(Vector2::new(world_x, world_z));
                let block = world.get_block(&BlockPos::new(world_x, top_y, world_z));

                let color_base = block.map_color;

                let mut brightness = 2; // Normal
                if prev_y != -1 {
                    if top_y > prev_y {
                        brightness = 3; // High
                    } else if top_y < prev_y {
                        brightness = 1; // Low
                    }
                }
                prev_y = top_y;

                let color = color_base * 4 + brightness;
                self.set_color(img_x, img_z, color);
            }
        }
    }
}

#[derive(Clone)]
pub struct MapDecoration {
    pub icon_type: i32,
    pub x: i8,
    pub z: i8,
    pub direction: i8,
    pub display_name: Option<pumpkin_util::text::TextComponent>,
}
