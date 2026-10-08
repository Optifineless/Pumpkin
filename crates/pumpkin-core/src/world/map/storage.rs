use super::{MAX_SCALE, MapData, MapManager};
use pumpkin_data::dimension::Dimension;
use pumpkin_nbt::{Nbt, compound::NbtCompound, deserializer::NbtReadHelperJava, tag::NbtTag};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

const MAX_MAP_BYTES: u64 = 1024 * 1024;

impl MapManager {
    /// Loads `MapIndex` and scans stored IDs without decompressing archived maps.
    pub fn load(world: &Path) -> io::Result<Self> {
        let mut manager = Self::new();
        manager.storage_path = Some(world.to_owned());
        for path in [
            "data/idcounts.dat",
            "data/minecraft/idcounts.dat",
            "data/minecraft/maps/last_id.dat",
        ] {
            match read_root(&world.join(path)) {
                Ok(root) => {
                    // MapIndex.CODEC stores the last allocated ID, unlike Pumpkin's next ID.
                    let last = root
                        .get_compound("data")
                        .and_then(|data| data.get_int("map"))
                        .unwrap_or(-1);
                    manager.reconcile_counter(last.wrapping_add(1));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        // MapId.key / SavedDataStorage.getDataFile use minecraft:maps/<id> in 26.3.
        for (folder, legacy) in [
            (world.join("data"), true),
            (world.join("data/minecraft"), true),
            (world.join("data/minecraft/maps"), false),
        ] {
            let entries = match fs::read_dir(folder) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let Some(id) = name
                    .to_str()
                    .and_then(|s| {
                        if legacy {
                            s.strip_prefix("map_")
                        } else {
                            Some(s)
                        }
                    })
                    .and_then(|s| s.strip_suffix(".dat"))
                    .and_then(|s| s.parse::<i32>().ok())
                else {
                    continue;
                };
                // DimensionStorageFileFix migrates idcounts and map_<id> to maps/last_id and maps/<id>.
                manager.reserve_map_id(id);
            }
        }
        Ok(manager)
    }

    /// Saves the cache without holding map locks during disk I/O.
    pub async fn save(&self, world: &Path, data_version: i32) -> io::Result<()> {
        let gate = self.save_gate.clone().lock_owned().await;
        let maps: Vec<_> = self
            .maps
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();
        let folder = world.join("data/minecraft/maps");
        let next_id = self.next_counter();
        #[cfg(test)]
        let barrier = self
            .save_barrier
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            fs::create_dir_all(&folder)?;
            for (id, map) in maps {
                let root = {
                    let map = map
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let root = map.to_saved_nbt(data_version);
                    // Persistence state is independent of the pixel-packet dirty flag.
                    if map.saved_tag.as_ref() == Some(&root) {
                        continue;
                    }
                    root
                };
                let path = folder.join(format!("{id}.dat"));
                #[cfg(test)]
                if let Some(barrier) = &barrier {
                    barrier.wait();
                    barrier.wait();
                }
                write_root(&path, root.clone())?;
                // If the map changed during I/O, it still differs from this snapshot next save.
                map.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .saved_tag = Some(root);
            }
            let mut data = NbtCompound::new();
            data.put_int("map", next_id.wrapping_sub(1));
            let mut root = NbtCompound::new();
            root.put_compound("data", data);
            root.put_int("DataVersion", data_version);
            write_root(&folder.join("last_id.dat"), root)?;
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}

pub(super) fn read_map(path: &Path) -> io::Result<MapData> {
    let root = read_root(path)?;
    let mut map =
        MapData::from_saved_nbt(&root).ok_or_else(|| io::Error::other("Invalid map data"))?;
    map.saved_tag = Some(map.to_saved_nbt(root.get_int("DataVersion").unwrap_or(0)));
    Ok(map)
}

fn write_root(path: &Path, root: NbtCompound) -> io::Result<()> {
    let temporary = path.with_extension("dat.tmp");
    let file = fs::File::create(&temporary)?;
    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, &file).map_err(io::Error::other)?;
    file.sync_all()?;
    drop(file);
    pumpkin_world::replace(&temporary, path)?;
    pumpkin_world::sync_parent(path)
}

fn read_root(path: &Path) -> io::Result<NbtCompound> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_MAP_BYTES {
        return Err(io::Error::other("Map file is too large"));
    }
    let mut input = io::BufReader::new(file);
    let mut magic = [0; 2];
    input.read_exact(&mut magic)?;
    let input = io::Cursor::new(magic).chain(input);
    let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] {
        Box::new(flate2::read::GzDecoder::new(input))
    } else {
        Box::new(input)
    };
    let mut bytes = Vec::new();
    reader.take(MAX_MAP_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MAP_BYTES {
        return Err(io::Error::other("Map data is too large"));
    }
    let root = Nbt::read(&mut NbtReadHelperJava::new(&mut io::Cursor::new(bytes)))
        .map_err(io::Error::other)?;
    Ok(root.root_tag)
}

impl MapData {
    fn from_saved_nbt(root: &NbtCompound) -> Option<Self> {
        // MapItemSavedData.CODEC clamps scale and ignores color arrays of the wrong length.
        let data = root.get_compound("data")?;
        let dimension = Dimension::from_name(data.get_string("dimension")?)?.clone();
        let mut map = Self::new(
            dimension,
            data.get_int("xCenter")?,
            data.get_int("zCenter")?,
            data.get_byte("scale").unwrap_or(0).clamp(0, MAX_SCALE),
        );
        let colors = data.get_byte_array("colors")?;
        if colors.len() == map.colors.len() {
            for (target, value) in map.colors.iter_mut().zip(colors) {
                *target = *value as u8;
            }
        }
        map.locked = data.get_bool("locked").unwrap_or(false);
        map.tracking_position = data.get_bool("trackingPosition").unwrap_or(true);
        map.unlimited_tracking = data.get_bool("unlimitedTracking").unwrap_or(false);
        map.banners = data.get_list("banners").unwrap_or_default().to_vec();
        map.frames = data.get_list("frames").unwrap_or_default().to_vec();
        map.rebuild_markers();
        Some(map)
    }

    fn to_saved_nbt(&self, data_version: i32) -> NbtCompound {
        // SavedDataStorage.collectDirtyTagsToSave wraps MapItemSavedData.CODEC in data/DataVersion.
        let mut data = NbtCompound::new();
        data.put_string("dimension", self.dimension.minecraft_name.to_owned());
        data.put_int("xCenter", self.center_x);
        data.put_int("zCenter", self.center_z);
        data.put_byte("scale", self.scale);
        data.put(
            "colors",
            NbtTag::ByteArray(self.colors.iter().map(|v| *v as i8).collect()),
        );
        data.put_bool("locked", self.locked);
        data.put_bool("trackingPosition", self.tracking_position);
        data.put_bool("unlimitedTracking", self.unlimited_tracking);
        data.put_list("banners", self.banners.clone());
        data.put_list("frames", self.frames.clone());
        let mut root = NbtCompound::new();
        root.put_compound("data", data);
        root.put_int("DataVersion", data_version);
        root
    }
}
