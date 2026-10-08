use crate::chunk::ChunkParsingError;
use pumpkin_data::dimension::Dimension;
use pumpkin_nbt::compound::NbtCompound;

pub(super) struct SectionBounds {
    pub min: i32,
    pub count: usize,
}

fn invalid() -> ChunkParsingError {
    ChunkParsingError::ErrorDeserializingChunk("Chunk section outside dimension bounds".into())
}

impl SectionBounds {
    pub fn new(root: &NbtCompound, dimension: &Dimension) -> Result<Self, ChunkParsingError> {
        let min = dimension.min_y.div_euclid(16);
        let count = usize::try_from(dimension.height.div_euclid(16)).map_err(|_| invalid())?;
        if root.has("yPos") && root.get_int("yPos") != Some(min) {
            return Err(invalid());
        }
        Ok(Self { min, count })
    }

    pub fn index(&self, y: i32, section: &NbtCompound) -> Result<Option<usize>, ChunkParsingError> {
        let index = i64::from(y) - i64::from(self.min);
        if (0..self.count as i64).contains(&index) {
            return Ok(Some(index as usize));
        }
        // SerializableChunkData.parse keeps boundary light sections outside the block allocation.
        if (index == -1 || index == self.count as i64)
            && !section.has("block_states")
            && !section.has("biomes")
        {
            return Ok(None);
        }
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkData, format::anvil::SingleChunkDataSerializer};
    use pumpkin_nbt::{Nbt, tag::NbtTag};
    use pumpkin_util::math::vector2::Vector2;

    #[test]
    fn rejects_disk_bounds_before_allocating_and_accepts_light_boundaries() {
        let mut root = NbtCompound::new();
        root.put_int("xPos", 0);
        root.put_int("zPos", 0);
        root.put_string("Status", "minecraft:full".into());
        root.put_int("yPos", i32::MIN);
        let parse =
            |root: NbtCompound| ChunkData::from_bytes(&Nbt::from(root).write(), Vector2::new(0, 0));
        assert!(parse(root.clone()).is_err());
        root.put_long("yPos", i64::MAX);
        assert!(parse(root.clone()).is_err());
        root.put_int("yPos", -4);
        let mut section = NbtCompound::new();
        section.put_int("Y", i32::MAX);
        root.put(
            "sections",
            NbtTag::List(vec![NbtTag::Compound(section.clone())]),
        );
        assert!(parse(root.clone()).is_err());
        section.put_long("Y", i64::MAX);
        root.put(
            "sections",
            NbtTag::List(vec![NbtTag::Compound(section.clone())]),
        );
        assert!(parse(root.clone()).is_err());
        section.put_int("Y", -5);
        root.put("sections", NbtTag::List(vec![NbtTag::Compound(section)]));
        let chunk = parse(root).unwrap();
        assert_eq!(chunk.section.min_y, -64);
        assert_eq!(chunk.section.count, 24);
    }

    #[test]
    fn nether_bounds_come_from_requested_dimension() {
        let mut root = NbtCompound::new();
        root.put_int("xPos", 0);
        root.put_int("zPos", 0);
        root.put_int("yPos", 0);
        root.put_string("Status", "minecraft:full".into());
        let chunk = ChunkData::from_bytes_in_dimension(
            &Nbt::from(root).write(),
            Vector2::new(0, 0),
            &Dimension::THE_NETHER,
        )
        .unwrap();
        assert_eq!(chunk.section.min_y, Dimension::THE_NETHER.min_y);
        assert_eq!(
            chunk.section.count as i32 * 16,
            Dimension::THE_NETHER.height
        );
    }
}
