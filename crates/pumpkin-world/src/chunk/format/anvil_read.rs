use super::{
    AnvilChunkData, AnvilChunkFile, AnvilChunkMetadata, Bytes, CHUNK_COUNT, ChunkReadingError,
    HEADER_SECTORS, SECTOR_BYTES, SingleChunkDataSerializer, region_write,
};

impl<S: SingleChunkDataSerializer> AnvilChunkFile<S> {
    // RegionFile's constructor zero-pads the header and ignores invalid locations.
    pub(super) fn read_region(bytes: &Bytes) -> Result<Self, ChunkReadingError> {
        let mut file = Self::default();
        file.reservations
            .get_mut()
            .initialize(bytes, bytes.len() as u64);
        let header = file.reservations.get_mut().header.clone();
        for index in 0..CHUNK_COUNT {
            let start = index * 4;
            let location = u32::from_be_bytes(
                header[start..start + 4]
                    .try_into()
                    .map_err(|_| ChunkReadingError::InvalidHeader)?,
            );
            if location == 0 {
                continue;
            }
            match Self::read_record(bytes, index, location) {
                Ok(metadata) => file.chunks_data[index] = Some(metadata),
                Err(error) => {
                    file.read_errors.insert(index, error.to_string());
                }
            }
        }
        Ok(file)
    }

    fn read_record(
        bytes: &Bytes,
        index: usize,
        location: u32,
    ) -> Result<AnvilChunkMetadata, ChunkReadingError> {
        let count = (location & 0xff) as usize;
        let offset = (location >> 8) as usize;
        if offset < HEADER_SECTORS
            || count == 0
            || offset + count > region_write::MAX_SECTOR_OFFSET + 1
        {
            return Err(ChunkReadingError::InvalidHeader);
        }
        let start = offset * SECTOR_BYTES;
        let end = ((offset + count) * SECTOR_BYTES).min(bytes.len());
        if start >= end {
            return Err(ChunkReadingError::InvalidHeader);
        }
        // RegionFile.getChunkDataInputStream accepts absent padding, never absent payload.
        let serialized_data = AnvilChunkData::from_bytes(bytes.slice(start..end))?;
        let timestamp_start = SECTOR_BYTES + index * 4;
        let timestamp = bytes
            .get(timestamp_start..timestamp_start + 4)
            .map_or(0, |value| {
                u32::from_be_bytes([value[0], value[1], value[2], value[3]])
            });
        Ok(AnvilChunkMetadata {
            serialized_data,
            timestamp,
        })
    }
}
