use super::{MapData, MapDecoration};
use pumpkin_data::{data_component_impl::CustomNameImpl, map_decoration::MapDecorationType};
use pumpkin_nbt::tag::NbtTag;

impl MapData {
    pub(super) fn rebuild_markers(&mut self) {
        // MapItemSavedData's loading constructor calls addDecoration for banners and frames.
        self.decorations.clear();
        let mut indices = std::collections::HashMap::new();
        for banner in self.banners.clone() {
            let Some(banner) = banner.extract_compound() else {
                continue;
            };
            let Some(pos) = banner.get("pos").and_then(marker_position) else {
                continue;
            };
            let color = banner.get_string("color").unwrap_or("white");
            let kind = MapDecorationType::from_name(&format!("banner_{color}"))
                .unwrap_or(&MapDecorationType::BANNER_WHITE);
            let name = banner
                .get("name")
                .and_then(CustomNameImpl::read_data)
                .map(|name| name.name);
            if let Some(marker) = self.marker(*kind, pos, 180, name) {
                let key = format!("banner-{},{},{}", pos[0], pos[1], pos[2]);
                self.add_marker(&mut indices, key, marker);
            }
        }
        for frame in self.frames.clone() {
            let Some(frame) = frame.extract_compound() else {
                continue;
            };
            let Some(pos) = frame.get("pos").and_then(marker_position) else {
                continue;
            };
            let Some(entity) = frame.get_int("entity_id") else {
                continue;
            };
            if let Some(marker) = self.marker(
                MapDecorationType::FRAME,
                pos,
                frame.get_int("rotation").unwrap_or(0),
                None,
            ) {
                self.add_marker(&mut indices, format!("frame-{entity}"), marker);
            }
        }
    }

    fn add_marker(
        &mut self,
        indices: &mut std::collections::HashMap<String, usize>,
        key: String,
        marker: MapDecoration,
    ) {
        // MapItemSavedData.addDecoration replaces a keyed entry without changing insertion order.
        if let Some(index) = indices.get(&key) {
            self.decorations[*index] = marker;
        } else {
            indices.insert(key, self.decorations.len());
            self.decorations.push(marker);
        }
    }

    fn marker(
        &self,
        kind: MapDecorationType,
        pos: [i32; 3],
        rotation: i32,
        name: Option<pumpkin_util::text::TextComponent>,
    ) -> Option<MapDecoration> {
        // MapItemSavedData.calculateDecorationLocationAndType / clampMapCoordinate / calculateRotation.
        const HALF_SIZE: f32 = 63.0;
        let scale = (1 << self.scale) as f32;
        let x = (i64::from(pos[0]) - i64::from(self.center_x)) as f32 / scale;
        let z = (i64::from(pos[2]) - i64::from(self.center_z)) as f32 / scale;
        if !self.unlimited_tracking && (x.abs() > HALF_SIZE || z.abs() > HALF_SIZE) {
            return None;
        }
        let coordinate = |v: f32| {
            if v <= -HALF_SIZE {
                i8::MIN
            } else if v >= HALF_SIZE {
                i8::MAX
            } else {
                (v * 2.0 + 0.5) as i8
            }
        };
        let adjusted = if rotation < 0 {
            f64::from(rotation) - 8.0
        } else {
            f64::from(rotation) + 8.0
        };
        Some(MapDecoration {
            icon_type: kind.id as i32,
            x: coordinate(x),
            z: coordinate(z),
            direction: (adjusted * 16.0 / 360.0) as i32 as i8,
            display_name: name,
        })
    }
}

fn marker_position(tag: &NbtTag) -> Option<[i32; 3]> {
    if let NbtTag::IntArray(values) = tag {
        values.as_slice().try_into().ok()
    } else {
        let values = tag.extract_list()?;
        Some([
            values.first()?.extract_int()?,
            values.get(1)?.extract_int()?,
            values.get(2)?.extract_int()?,
        ])
    }
}
