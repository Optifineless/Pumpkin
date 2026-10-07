use bytes::{Buf, Bytes};
use flate2::read::{GzDecoder, GzEncoder, ZlibDecoder, ZlibEncoder};
use lz4_java_wrc::Context;
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_util::math::vector2::Vector2;
use std::{
    io::{Read, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::Mutex,
};
use tracing::debug;

use crate::chunk::{
    ChunkReadingError, ChunkSerializingError, ChunkWritingError, CompressionError,
    io::{ChunkSerializer, Dirtiable, LoadedData, run_blocking},
};

/// The side size of a region in chunks (one region is 32x32 chunks)
pub const REGION_SIZE: usize = 32;

/// The number of bits that identify two chunks in the same region
pub const SUBREGION_BITS: u8 = pumpkin_util::math::ceil_log2(REGION_SIZE as u32);

pub const SUBREGION_AND: i32 = i32::pow(2, SUBREGION_BITS as u32) - 1;

/// The number of chunks in a region
pub const CHUNK_COUNT: usize = REGION_SIZE * REGION_SIZE;

/// The number of bytes in a sector (4 KiB)
const SECTOR_BYTES: usize = 4096;

// RegionFile's stream header and external-stream constants.
const CHUNK_HEADER_SIZE: usize = 5;
const EXTERNAL_STREAM_FLAG: u8 = 128;
const EXTERNAL_CHUNK_THRESHOLD: usize = 256;
const HEADER_SECTORS: usize = 2;

// 26.2
pub const WORLD_DATA_VERSION: i32 = 4903;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    /// `GZip` Compression
    GZip = Self::GZIP_ID,
    /// `ZLib` Compression
    ZLib = Self::ZLIB_ID,
    /// LZ4 Compression (since 24w04a)
    LZ4 = Self::LZ4_ID,
    /// Custom compression algorithm (since 24w05a)
    Custom = Self::CUSTOM_ID,
}

pub enum CompressionRead<R: Read> {
    GZip(GzDecoder<R>),
    ZLib(ZlibDecoder<R>),
    LZ4(lz4_java_wrc::Lz4BlockInput<R>),
}

impl<R: Read> Read for CompressionRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::GZip(gzip) => gzip.read(buf),
            Self::ZLib(zlib) => zlib.read(buf),
            Self::LZ4(lz4) => lz4.read(buf),
        }
    }
}

#[derive(Clone)]
pub struct AnvilChunkData {
    compression: Option<Compression>,
    // Length is always the length of this + compression byte (1) so we dont need to save a length
    compressed_data: Bytes,
    external: bool,
    external_path: Option<PathBuf>,
}

enum WriteAction {
    // Don't write anything
    Pass,
    // Only write certain indices
    Parts(Vec<usize>),
}

impl WriteAction {
    /// If we are currently not writing, sets to new Parts enum,
    /// If we have parts enum, add to it,
    fn maybe_update_chunk_index(&mut self, index: usize) {
        match self {
            Self::Pass => *self = Self::Parts(vec![index]),
            Self::Parts(parts) => {
                if !parts.contains(&index) {
                    parts.push(index);
                }
            }
        }
    }
}

struct AnvilChunkMetadata {
    serialized_data: AnvilChunkData,
    timestamp: u32,
}

pub struct AnvilChunkFile<S: SingleChunkDataSerializer> {
    chunks_data: Box<[Option<AnvilChunkMetadata>]>,
    write_action: Mutex<WriteAction>,
    reservations: Mutex<region_write::RegionBitmap>,
    read_errors: std::collections::BTreeMap<usize, String>,
    #[cfg(test)]
    fail_before_header: std::sync::atomic::AtomicBool,

    #[cfg(test)]
    fail_header_write: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    fail_header_sync: std::sync::atomic::AtomicBool,
    _dummy: PhantomData<S>,
}

impl Compression {
    const GZIP_ID: u8 = 1;
    const ZLIB_ID: u8 = 2;
    const NO_COMPRESSION_ID: u8 = 3;
    const LZ4_ID: u8 = 4;
    const CUSTOM_ID: u8 = 127;

    fn decompress_data(self, compressed_data: &[u8]) -> Result<Box<[u8]>, CompressionError> {
        fn decode<R: std::io::Read>(mut reader: R, capacity: usize) -> std::io::Result<Box<[u8]>> {
            let mut buf = Vec::with_capacity(capacity);
            reader.read_to_end(&mut buf)?;
            Ok(buf.into_boxed_slice())
        }

        let initial_capacity = compressed_data.len();

        match self {
            Self::GZip => decode(GzDecoder::new(compressed_data), initial_capacity)
                .map_err(CompressionError::GZipError),
            Self::ZLib => decode(ZlibDecoder::new(compressed_data), initial_capacity)
                .map_err(CompressionError::ZlibError),
            Self::LZ4 => decode(
                lz4_java_wrc::Lz4BlockInput::new(compressed_data),
                initial_capacity,
            )
            .map_err(CompressionError::LZ4Error),
            Self::Custom => Err(CompressionError::UnknownCompression),
        }
    }

    const LZ4_COMPRESSION_LEVEL_BASE: u32 = 10;
    fn compress_data(
        self,
        uncompressed_data: &[u8],
        compression_level: u32,
    ) -> Result<Vec<u8>, CompressionError> {
        match self {
            Self::GZip => {
                let mut encoder = GzEncoder::new(
                    uncompressed_data,
                    flate2::Compression::new(compression_level),
                );
                let mut chunk_data = Vec::new();
                encoder
                    .read_to_end(&mut chunk_data)
                    .map_err(CompressionError::GZipError)?;
                Ok(chunk_data)
            }
            Self::ZLib => {
                let mut encoder = ZlibEncoder::new(
                    uncompressed_data,
                    flate2::Compression::new(compression_level),
                );
                let mut chunk_data = Vec::new();
                encoder
                    .read_to_end(&mut chunk_data)
                    .map_err(CompressionError::ZlibError)?;
                Ok(chunk_data)
            }
            Self::LZ4 => {
                let mut compressed_data = Vec::new();
                let block_size = 1 << (Self::LZ4_COMPRESSION_LEVEL_BASE + compression_level);
                let mut encoder = lz4_java_wrc::Lz4BlockOutput::with_context(
                    &mut compressed_data,
                    Context::default(),
                    block_size,
                )
                .map_err(CompressionError::LZ4Error)?;
                encoder
                    .write_all(uncompressed_data)
                    .map_err(CompressionError::LZ4Error)?;
                drop(encoder);
                Ok(compressed_data)
            }
            Self::Custom => Err(CompressionError::UnknownCompression),
        }
    }

    /// Returns Ok when a compression is found otherwise an Err
    #[expect(clippy::result_unit_err)]
    pub const fn from_byte(byte: u8) -> Result<Option<Self>, ()> {
        match byte {
            Self::GZIP_ID => Ok(Some(Self::GZip)),
            Self::ZLIB_ID => Ok(Some(Self::ZLib)),
            // Uncompressed (since a version before 1.15.1)
            Self::NO_COMPRESSION_ID => Ok(None),
            Self::LZ4_ID => Ok(Some(Self::LZ4)),
            Self::CUSTOM_ID => Ok(Some(Self::Custom)),
            // Unknown format
            _ => Err(()),
        }
    }
}

impl From<pumpkin_config::chunk::Compression> for Compression {
    fn from(value: pumpkin_config::chunk::Compression) -> Self {
        // :c
        match value {
            pumpkin_config::chunk::Compression::GZip => Self::GZip,
            pumpkin_config::chunk::Compression::ZLib => Self::ZLib,
            pumpkin_config::chunk::Compression::LZ4 => Self::LZ4,
            pumpkin_config::chunk::Compression::Custom => Self::Custom,
        }
    }
}

impl AnvilChunkData {
    /// Raw size of serialized chunk
    #[inline]
    const fn raw_write_size(&self) -> usize {
        // 4 bytes for the *length* and 1 byte for the *compression* method
        self.compressed_data.len() + CHUNK_HEADER_SIZE
    }

    // RegionFile.write externalizes streams needing at least 256 sectors.
    const fn is_external(&self) -> bool {
        self.external || self.raw_write_size().div_ceil(SECTOR_BYTES) >= EXTERNAL_CHUNK_THRESHOLD
    }

    const fn sector_count(&self) -> u32 {
        if self.is_external() {
            1
        } else {
            self.raw_write_size().div_ceil(SECTOR_BYTES) as u32
        }
    }

    fn from_bytes(mut bytes: Bytes) -> Result<Self, ChunkReadingError> {
        if bytes.len() < CHUNK_HEADER_SIZE {
            return Err(ChunkReadingError::InvalidHeader);
        }
        let declared_length = bytes.get_u32() as usize;
        let compression_method = bytes.get_u8();
        let external = compression_method & EXTERNAL_STREAM_FLAG != 0;
        let Some(length) = declared_length.checked_sub(1) else {
            return Err(ChunkReadingError::InvalidHeader);
        };
        if !external && length > bytes.len() {
            return Err(ChunkReadingError::InvalidHeader);
        }
        // RegionFile.getChunkDataInputStream follows the external flag even for a mixed stream.
        if external && length != 0 {
            tracing::warn!("Chunk has both internal and external streams");
        }
        let compression = Compression::from_byte(compression_method & !EXTERNAL_STREAM_FLAG)
            .map_err(|()| ChunkReadingError::Compression(CompressionError::UnknownCompression))?;
        Ok(Self {
            compression,
            compressed_data: if external {
                Bytes::new()
            } else {
                bytes.slice(..length)
            },
            external,
            external_path: None,
        })
    }

    async fn write(&self, w: &mut (impl AsyncWrite + Unpin + Send)) -> std::io::Result<()> {
        static PADDING: [u8; SECTOR_BYTES] = [0; SECTOR_BYTES];
        let compression = self
            .compression
            .map_or(Compression::NO_COMPRESSION_ID, |c| c as u8);
        let raw_size = if self.is_external() {
            // RegionFile.createExternalStub: length 1 followed by the flagged compression ID.
            w.write_u32(1).await?;
            w.write_u8(compression | EXTERNAL_STREAM_FLAG).await?;
            CHUNK_HEADER_SIZE
        } else {
            w.write_u32((self.compressed_data.len() + 1) as u32).await?;
            w.write_u8(compression).await?;
            w.write_all(&self.compressed_data).await?;
            self.raw_write_size()
        };
        let padding_len = self.sector_count() as usize * SECTOR_BYTES - raw_size;
        w.write_all(&PADDING[..padding_len]).await
    }

    #[cfg(test)]
    fn to_chunk<S: SingleChunkDataSerializer>(
        &self,
        pos: Vector2<i32>,
    ) -> Result<S, ChunkReadingError> {
        self.to_chunk_in_dimension(pos, &pumpkin_data::dimension::Dimension::OVERWORLD)
    }

    fn to_chunk_in_dimension<S>(
        &self,
        pos: Vector2<i32>,
        dimension: &pumpkin_data::dimension::Dimension,
    ) -> Result<S, ChunkReadingError>
    where
        S: SingleChunkDataSerializer,
    {
        if self.external && self.compressed_data.is_empty() {
            let path = self
                .external_path
                .as_ref()
                .ok_or(ChunkReadingError::InvalidHeader)?;
            // RegionFile.createExternalChunkInputStream opens only the requested sidecar.
            let file = std::fs::File::open(path).map_err(ChunkReadingError::IoError)?;
            let mut bytes = Vec::new();
            file.take(region_write::MAX_EXTERNAL_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(ChunkReadingError::IoError)?;
            if bytes.len() as u64 > region_write::MAX_EXTERNAL_BYTES {
                return Err(ChunkReadingError::InvalidHeader);
            }
            let mut data = self.clone();
            data.compressed_data = bytes.into();
            data.external = false;
            return data.to_chunk_in_dimension(pos, dimension);
        }
        if let Some(compression) = self.compression {
            let decompress_bytes = compression
                .decompress_data(&self.compressed_data)
                .map_err(ChunkReadingError::Compression)?;

            S::from_bytes_in_dimension(&decompress_bytes.into(), pos, dimension)
        } else {
            S::from_bytes_in_dimension(&self.compressed_data, pos, dimension)
        }
    }

    fn from_chunk<S>(
        chunk: &S,
        compression: Option<Compression>,
        chunk_config: &AnvilChunkConfig,
    ) -> Result<Self, ChunkWritingError>
    where
        S: SingleChunkDataSerializer,
    {
        let raw_bytes = chunk
            .to_bytes()
            .map_err(|err| ChunkWritingError::ChunkSerializingError(err.to_string()))?;

        let compressed_data = match compression {
            Some(compression) => compression
                .compress_data(&raw_bytes, chunk_config.compression.level)
                .map_err(ChunkWritingError::Compression)?
                .into(),
            None => raw_bytes,
        };

        Ok(Self {
            compression,
            compressed_data,
            external: false,
            external_path: None,
        })
    }
}

impl<S: SingleChunkDataSerializer> AnvilChunkFile<S> {
    #[must_use]
    pub const fn get_region_coords(at: &Vector2<i32>) -> (i32, i32) {
        // Divide by 32 for the region coordinates
        (at.x >> SUBREGION_BITS, at.y >> SUBREGION_BITS)
    }

    #[must_use]
    pub const fn get_chunk_index(x: i32, z: i32) -> usize {
        let local_x = x & SUBREGION_AND;
        let local_z = z & SUBREGION_AND;
        let index = (local_z << SUBREGION_BITS) + local_x;
        index as usize
    }
}

impl<S: SingleChunkDataSerializer> Default for AnvilChunkFile<S> {
    fn default() -> Self {
        Self {
            chunks_data: (0..CHUNK_COUNT).map(|_| None).collect(),
            write_action: Mutex::new(WriteAction::Pass),
            reservations: Mutex::new(region_write::RegionBitmap::default()),
            #[cfg(test)]
            fail_header_write: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_header_sync: std::sync::atomic::AtomicBool::new(false),
            read_errors: std::collections::BTreeMap::new(),
            #[cfg(test)]
            fail_before_header: std::sync::atomic::AtomicBool::new(false),
            _dummy: PhantomData,
        }
    }
}

pub trait SingleChunkDataSerializer: Send + Sync + Sized + Dirtiable + 'static {
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError>;
    fn from_bytes(bytes: &Bytes, pos: Vector2<i32>) -> Result<Self, ChunkReadingError>;
    fn from_bytes_in_dimension(
        bytes: &Bytes,
        pos: Vector2<i32>,
        _dimension: &pumpkin_data::dimension::Dimension,
    ) -> Result<Self, ChunkReadingError> {
        Self::from_bytes(bytes, pos)
    }
    fn position(&self) -> (i32, i32);
}

impl<S: SingleChunkDataSerializer + 'static> ChunkSerializer for AnvilChunkFile<S> {
    type Data = S;
    type WriteBackend = PathBuf;

    type ChunkConfig = AnvilChunkConfig;

    fn should_write(&self, _is_watched: bool) -> bool {
        true
    }

    fn get_chunk_key(chunk: &Vector2<i32>) -> String {
        let (region_x, region_z) = Self::get_region_coords(chunk);
        format!("./r.{region_x}.{region_z}.mca")
    }

    async fn write(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        // Both legacy write_in_place settings use RegionFile's stable-inode protocol.
        // This retains vanilla's torn-header crash window, even for inline-only saves.
        let mut write_action = self.write_action.lock().await;
        match &*write_action {
            WriteAction::Pass => {
                debug!(
                    "Skipping write for {}, as there were no dirty chunks",
                    path.display()
                );
                Ok(())
            }
            WriteAction::Parts(parts) => self.write_indices(path, parts.iter().copied()).await,
        }?;

        // If we still are in memory after this, we don't need to write again!
        *write_action = WriteAction::Pass;
        Ok(())
    }

    fn read(r: Bytes) -> Result<Self, ChunkReadingError> {
        Self::read_region(&r)
    }

    fn read_with_path(r: Bytes, path: &Path) -> Result<Self, ChunkReadingError> {
        let mut file = Self::read(r)?;
        for (index, metadata) in file.chunks_data.iter_mut().enumerate() {
            if let Some(metadata) = metadata
                && metadata.serialized_data.external
            {
                metadata.serialized_data.external_path = Some(
                    region_write::external_path(path, index).map_err(ChunkReadingError::IoError)?,
                );
            }
        }
        Ok(file)
    }

    async fn update_chunk(
        &mut self,
        chunk: Arc<Self::Data>,
        chunk_config: &Self::ChunkConfig,
    ) -> Result<(), ChunkWritingError> {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        let index = Self::get_chunk_index(chunk.position().0, chunk.position().1);
        // Preserve even stream ID 3: sidecar-first publication must leave the old stub readable.
        let compression = self.chunks_data[index].as_ref().map_or_else(
            || Some(chunk_config.compression.algorithm.into()),
            |metadata| metadata.serialized_data.compression,
        );
        let config = chunk_config.clone();
        let serialized_data =
            run_blocking(move || AnvilChunkData::from_chunk(&*chunk, compression, &config))
                .await
                .map_err(|_| {
                    ChunkWritingError::IoError(std::io::Error::other(
                        "chunk serialization task failed",
                    ))
                })??;
        self.read_errors.remove(&index);
        self.chunks_data[index] = Some(AnvilChunkMetadata {
            serialized_data,
            timestamp: epoch,
        });
        let mut write_action = self.write_action.lock().await;
        // RegionFile.write always updates fresh sectors in the existing region inode.
        write_action.maybe_update_chunk_index(index);
        Ok(())
    }

    async fn get_chunks(
        &self,
        chunks: Vec<Vector2<i32>>,
        stream: tokio::sync::mpsc::Sender<LoadedData<Self::Data, ChunkReadingError>>,
        dimension: pumpkin_data::dimension::Dimension,
    ) {
        let chunk_items: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let index = Self::get_chunk_index(chunk.x, chunk.y);
                let data = self.chunks_data[index]
                    .as_ref()
                    .map(|chunk_metadata| chunk_metadata.serialized_data.clone());
                (chunk, data, self.read_errors.get(&index).cloned())
            })
            .collect();

        let (tx, mut rx) = tokio::sync::mpsc::channel(chunk_items.len().max(1));

        rayon::spawn(move || {
            use rayon::prelude::*;
            chunk_items
                .into_par_iter()
                .for_each(|(chunk, serialized_data, error)| {
                    if let Some(error) = error {
                        let _ = tx.blocking_send(LoadedData::Error((
                            chunk,
                            ChunkReadingError::IoError(std::io::Error::other(error)),
                        )));
                        return;
                    }
                    let result = serialized_data.map_or_else(
                        || LoadedData::Missing(chunk),
                        |data| match data.to_chunk_in_dimension(chunk, &dimension) {
                            Ok(chunk_res) => LoadedData::Loaded(chunk_res),
                            Err(err) => LoadedData::Error((chunk, err)),
                        },
                    );
                    let _ = tx.blocking_send(result);
                });
        });

        while let Some(item) = rx.recv().await {
            if stream.send(item).await.is_err() {
                return;
            }
        }
    }
}

/*
#[cfg(test)]
mod tests {

    use pumpkin_config::{AdvancedConfiguration, advanced_config, override_config_for_testing};
    use pumpkin_data::BlockDirection;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_util::math::vector2::Vector2;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use temp_dir::TempDir;
    use tokio::sync::RwLock;

    use crate::chunk::ChunkData;
    use crate::chunk::format::anvil::{AnvilChunkFile, SingleChunkDataSerializer};
    use crate::chunk::io::file_manager::{ChunkFileManager, PathFromLevelFolder};
    use crate::chunk::io::{FileIO, LoadedData};
    use crate::dimension::Dimension;
    use crate::generation::{Seed, get_world_gen};
    use crate::level::{Level, LevelFolder, SyncChunk};
    use crate::world::{BlockAccessor, BlockRegistryExt};

    struct BlockRegistry;

    impl BlockRegistryExt for BlockRegistry {
        fn can_place_at(
            &self,
            _block: &pumpkin_data::Block,
            _block_accessor: &dyn BlockAccessor,
            _block_pos: &BlockPos,
            _face: BlockDirection,
        ) -> bool {
            true
        }
    }

    async fn get_chunks<S>(
        saver: &ChunkFileManager<AnvilChunkFile<S>>,
        folder: &LevelFolder,
        chunks: &[(Vector2<i32>, SyncChunk)],
    ) -> Box<[Arc<RwLock<S>>]>
    where
        S: SingleChunkDataSerializer + PathFromLevelFolder + 'static,
    {
        let mut read_chunks = Vec::new();
        let (send, mut recv) = tokio::sync::mpsc::channel(1);

        let chunk_pos = chunks.iter().map(|(at, _)| *at).collect::<Vec<_>>();
        let spawn = saver.fetch_chunks(folder, &chunk_pos, send);
        let collect = async {
            while let Some(data) = recv.recv().await {
                read_chunks.push(data);
            }
        };

        tokio::join!(spawn, collect);

        let read_chunks = read_chunks
            .into_iter()
            .map(|chunk| match chunk {
                LoadedData::Loaded(chunk) => chunk,
                LoadedData::Missing(_) => panic!("Missing chunk"),
                LoadedData::Error((position, error)) => {
                    panic!("Error reading chunk at {position:?} | Error: {error:?}")
                }
            })
            .collect::<Vec<_>>();

        read_chunks.into_boxed_slice()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn not_existing() {
        let region_path = PathBuf::from("not_existing");
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();

        let mut chunks = Vec::new();
        let (send, mut recv) = tokio::sync::mpsc::channel(1);

        chunk_saver
            .fetch_chunks(
                &LevelFolder {
                    root_folder: PathBuf::from(""),
                    region_folder: region_path,
                    entities_folder: PathBuf::from(""),
                },
                &[Vector2::new(0, 0)],
                send,
            )
            .await;

        while let Some(data) = recv.recv().await {
            chunks.push(data);
        }

        assert!(chunks.len() == 1 && matches!(chunks[0], LoadedData::Missing(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn write_in_place() {
        let mut config = AdvancedConfiguration::default();
        config.chunk.write_in_place = true;
        override_config_for_testing(config);
        assert!(advanced_config().chunk.write_in_place);

        let _ = env_logger::try_init();

        let generator = get_world_gen(Seed(0), Dimension::Overworld, false, Vec::new(), String::new());

        let temp_dir = TempDir::new().unwrap();
        let level_folder = LevelFolder {
            root_folder: temp_dir.path().to_path_buf(),
            region_folder: temp_dir.path().join("region"),
            entities_folder: PathBuf::from("entities"),
        };
        fs::create_dir(&level_folder.region_folder).expect("couldn't create region folder");
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let block_registry = Arc::new(BlockRegistry);

        // Generate chunks
        let mut chunks = vec![];
        let level = Arc::new(Level::from_root_folder(
            temp_dir.path().to_path_buf(),
            block_registry.clone(),
            0,
            Dimension::Overworld,
        ));
        for x in -5..5 {
            for y in -5..5 {
                let position = Vector2::new(x, y);
                let chunk = generator.generate_chunk(&level, block_registry.as_ref(), &position);
                chunks.push((position, Arc::new(RwLock::new(chunk))));
            }
        }

        // TEST APPEND TO END

        chunk_saver
            .save_chunks(&level_folder, chunks.clone())
            .await
            .expect("Failed to write chunk");

        // Create a new manager to ensure nothing is cached
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let read_chunks = get_chunks(&chunk_saver, &level_folder, &chunks).await;

        for (_, chunk) in &chunks {
            let chunk = chunk.read().await;
            for read_chunk in read_chunks.iter() {
                let read_chunk = read_chunk.read().await;
                if read_chunk.position == chunk.position {
                    let original = chunk.section.dump_blocks();
                    let read = read_chunk.section.dump_blocks();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    let original = chunk.section.dump_biomes();
                    let read = read_chunk.section.dump_biomes();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });
                    break;
                }
            }
        }

        // TEST WRITE IN PLACE

        // Idk what blocks these are, they just have to be different
        let mut chunk = chunks.first().unwrap().1.write().await;
        chunk.section.set_relative_block(0, 0, 0, 1000);
        // Mark dirty so we actually write it
        chunk.dirty = true;
        drop(chunk);
        let mut chunk = chunks.last().unwrap().1.write().await;
        chunk.section.set_relative_block(0, 0, 0, 1000);
        // Mark dirty so we actually write it
        chunk.dirty = true;
        drop(chunk);

        chunk_saver
            .save_chunks(&level_folder, chunks.clone())
            .await
            .expect("Failed to write chunk");

        // Create a new manager to ensure nothing is cached
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let read_chunks = get_chunks(&chunk_saver, &level_folder, &chunks).await;

        for (_, chunk) in &chunks {
            let chunk = chunk.read().await;
            for read_chunk in read_chunks.iter() {
                let read_chunk = read_chunk.read().await;
                if read_chunk.position == chunk.position {
                    let original = chunk.section.dump_blocks();
                    let read = read_chunk.section.dump_blocks();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    let original = chunk.section.dump_biomes();
                    let read = read_chunk.section.dump_biomes();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    break;
                }
            }
        }

        // TEST SWAP SHIFT

        // Make a big chunk
        let mut chunk = chunks.first().unwrap().1.write().await;
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..4 {
                    let block_id = 16 * 16 * y + 16 * z + x;
                    chunk.section.set_relative_block(x, y, z, block_id as u16);
                }
            }
        }
        // Mark dirty so we actually write it
        chunk.dirty = true;
        drop(chunk);
        let mut chunk = chunks[2].1.write().await;
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..4 {
                    let block_id = 16 * 16 * y + 16 * z + x;
                    chunk.section.set_relative_block(x, y, z, block_id as u16);
                }
            }
        }
        // Mark dirty so we actually write it
        chunk.dirty = true;
        drop(chunk);

        chunk_saver
            .save_chunks(&level_folder, chunks.clone())
            .await
            .expect("Failed to write chunk");

        // Create a new manager to ensure nothing is cached
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let read_chunks = get_chunks(&chunk_saver, &level_folder, &chunks).await;

        for (_, chunk) in &chunks {
            let chunk = chunk.read().await;
            for read_chunk in read_chunks.iter() {
                let read_chunk = read_chunk.read().await;
                if read_chunk.position == chunk.position {
                    let original = chunk.section.dump_blocks();
                    let read = read_chunk.section.dump_blocks();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    let original = chunk.section.dump_biomes();
                    let read = read_chunk.section.dump_biomes();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    break;
                }
            }
        }

        // TEST DEFAULT TO WRITE ALL

        // Make an even bigger chunk
        let mut chunk = chunks.last().unwrap().1.write().await;
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..16 {
                    let block_id = 16 * 16 * y + 16 * z + x;
                    chunk.section.set_relative_block(x, y, z, block_id as u16);
                }
            }
        }
        // Mark dirty so we actually write it
        chunk.dirty = true;
        drop(chunk);

        chunk_saver
            .save_chunks(&level_folder, chunks.clone())
            .await
            .expect("Failed to write chunk");

        // Create a new manager to ensure nothing is cached
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let read_chunks = get_chunks(&chunk_saver, &level_folder, &chunks).await;

        for (_, chunk) in &chunks {
            let chunk = chunk.read().await;
            for read_chunk in read_chunks.iter() {
                let read_chunk = read_chunk.read().await;
                if read_chunk.position == chunk.position {
                    let original = chunk.section.dump_blocks();
                    let read = read_chunk.section.dump_blocks();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });

                    let original = chunk.section.dump_biomes();
                    let read = read_chunk.section.dump_biomes();

                    original
                        .into_iter()
                        .zip(read)
                        .enumerate()
                        .for_each(|(i, (o, r))| {
                            if o != r {
                                panic!("Data miss-match expected {o}, got {r} ({i})");
                            }
                        });
                    break;
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn write_bulk() {
        let mut config = AdvancedConfiguration::default();
        config.chunk.write_in_place = false;
        override_config_for_testing(config);
        assert!(!advanced_config().chunk.write_in_place);

        let _ = env_logger::try_init();

        let generator = get_world_gen(Seed(0), Dimension::Overworld, false, Vec::new(), String::new());

        let temp_dir = TempDir::new().unwrap();
        let level_folder = LevelFolder {
            root_folder: temp_dir.path().to_path_buf(),
            region_folder: temp_dir.path().join("region"),
            entities_folder: PathBuf::from("entities"),
        };
        fs::create_dir(&level_folder.region_folder).expect("couldn't create region folder");
        let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
        let block_registry = Arc::new(BlockRegistry);

        // Generate chunks
        let mut chunks = vec![];
        let level = Arc::new(Level::from_root_folder(
            temp_dir.path().to_path_buf(),
            block_registry.clone(),
            0,
            Dimension::Overworld,
        ));
        for x in -5..5 {
            for y in -5..5 {
                let position = Vector2::new(x, y);
                let chunk = generator.generate_chunk(&level, block_registry.as_ref(), &position);
                chunks.push((position, Arc::new(RwLock::new(chunk))));
            }
        }

        for _ in 0..5 {
            // Mark the chunks as dirty so we save them again
            for (_, chunk) in &chunks {
                let mut chunk = chunk.write().await;
                chunk.dirty = true;
            }

            chunk_saver
                .save_chunks(&level_folder, chunks.clone())
                .await
                .expect("Failed to write chunk");

            // Create a new manager to ensure nothing is cached
            let chunk_saver = ChunkFileManager::<AnvilChunkFile<ChunkData>>::default();
            let read_chunks = get_chunks(&chunk_saver, &level_folder, &chunks).await;

            for (_, chunk) in &chunks {
                let chunk = chunk.read().await;
                for read_chunk in read_chunks.iter() {
                    let read_chunk = read_chunk.read().await;
                    if read_chunk.position == chunk.position {
                        let original = chunk.section.dump_blocks();
                        let read = read_chunk.section.dump_blocks();

                        original
                            .into_iter()
                            .zip(read)
                            .enumerate()
                            .for_each(|(i, (o, r))| {
                                if o != r {
                                    panic!("Data miss-match expected {o}, got {r} ({i})");
                                }
                            });

                        let original = chunk.section.dump_biomes();
                        let read = read_chunk.section.dump_biomes();

                        original
                            .into_iter()
                            .zip(read)
                            .enumerate()
                            .for_each(|(i, (o, r))| {
                                if o != r {
                                    panic!("Data miss-match expected {o}, got {r} ({i})");
                                }
                            });
                        break;
                    }
                }
            }
        }
    }

    // TODO
    /*
    #[test]
    fn load_java_chunk() {
        let temp_dir = TempDir::new().unwrap();
        let level_folder = LevelFolder {
            root_folder: temp_dir.path().to_path_buf(),
            region_folder: temp_dir.path().join("region"),
        };

        fs::create_dir(&level_folder.region_folder).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(file!())
                .parent()
                .unwrap()
                .join("../../assets/r.0.0.mca"),
            level_folder.region_folder.join("r.0.0.mca"),
        )
        .unwrap();

        let mut actually_tested = false;
        for x in 0..(1 << 5) {
            for z in 0..(1 << 5) {
                let result = AnvilChunkFormat {}.read_chunk(&level_folder, &Vector2 { x, z });

                match result {
                    Ok(_) => actually_tested = true,
                    Err(ChunkReadingError::ParsingError(ChunkParsingError::ChunkNotGenerated)) => {}
                    Err(ChunkReadingError::ChunkNotExist) => {}
                    Err(e) => panic!("{:?}", e),
                }

                println!("=========== OK ===========");
            }
        }

        assert!(actually_tested);
    }
    */
}
 */
#[cfg(test)]
mod tests {
    use super::{AnvilChunkFile, Compression, CompressionError, SECTOR_BYTES};
    use crate::chunk::ChunkData;
    use crate::chunk::io::ChunkSerializer;
    use bytes::{BufMut, Bytes, BytesMut};

    /// A region file whose first location entry is `location`, whose other
    /// 1023 entries are absent, and whose single payload sector starts with
    /// `declared_len` followed by the "no compression" marker.
    fn region_with_first_location(location: u32, declared_len: u32) -> Bytes {
        let mut buf = BytesMut::with_capacity(SECTOR_BYTES * 3);
        buf.put_u32(location);
        buf.put_bytes(0, SECTOR_BYTES - 4);
        buf.put_bytes(0, SECTOR_BYTES);

        buf.put_u32(declared_len);
        buf.put_u8(3);
        buf.put_bytes(0, SECTOR_BYTES - 5);

        buf.freeze()
    }

    #[test]
    fn a_chunk_pointing_into_the_header_is_skipped() {
        // Sectors 0 and 1 hold the location and timestamp tables, so a chunk
        // cannot start before sector 2. Offset 0 is already skipped; offset 1
        // is just as impossible and used to be subtracted from anyway.
        let file = AnvilChunkFile::<ChunkData>::read(region_with_first_location((1 << 8) | 1, 1))
            .expect("one bad location entry should not fail the whole region");

        assert!(file.chunks_data[0].is_none());
    }

    #[test]
    fn a_chunk_at_the_first_free_sector_is_read() {
        // The control for the test above: sector 2 is the first legal one, and
        // a chunk there must not be skipped.
        let file = AnvilChunkFile::<ChunkData>::read(region_with_first_location((2 << 8) | 1, 1))
            .expect("a chunk at the first free sector is well formed");

        assert!(file.chunks_data[0].is_some());
    }

    #[test]
    fn a_chunk_declaring_no_length_is_an_error_not_a_panic() {
        // The length covers the compression byte, so the smallest legal value
        // is 1 and the byte count is that minus one. Zero came from a file, so
        // it has to be rejected rather than subtracted from.
        let file = AnvilChunkFile::<ChunkData>::read(region_with_first_location((2 << 8) | 1, 0));

        assert!(file.unwrap().read_errors.contains_key(&0));
    }

    #[test]
    fn a_chunk_declaring_one_byte_too_many_is_an_error_not_a_panic() {
        // RegionFile.getChunkDataInputStream subtracts the consumed compression byte.
        // The storage merge keeps this record error isolated from healthy chunks.
        let file = AnvilChunkFile::<ChunkData>::read(region_with_first_location(
            (2 << 8) | 1,
            SECTOR_BYTES as u32 - 3,
        ));

        assert!(file.unwrap().read_errors.contains_key(&0));
    }

    #[test]
    fn custom_compression_returns_unknown_compression_error() {
        assert!(matches!(
            Compression::Custom.compress_data(b"chunk data", 6),
            Err(CompressionError::UnknownCompression)
        ));
    }

    #[test]
    fn custom_decompression_returns_unknown_compression_error() {
        assert!(matches!(
            Compression::Custom.decompress_data(b"chunk data"),
            Err(CompressionError::UnknownCompression)
        ));
    }
}

#[path = "anvil_write.rs"]
mod region_write;

#[cfg(test)]
#[path = "anvil_tests.rs"]
mod storage_tests;

#[path = "anvil_read.rs"]
mod region_read;

#[cfg(test)]
#[path = "anvil_failure_tests.rs"]
mod failure_tests;
