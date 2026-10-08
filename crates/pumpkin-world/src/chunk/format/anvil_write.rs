use std::{
    io,
    path::{Path, PathBuf},
};

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::{AnvilChunkFile, HEADER_SECTORS, REGION_SIZE, SECTOR_BYTES, SingleChunkDataSerializer};
use crate::storage::{TemporaryFile, sync_parent_async};

// RegionFile.getSectorNumber/getNumSectors use 24 offset bits and 8 count bits.
pub(super) const MAX_SECTOR_OFFSET: usize = 0x00ff_ffff;
pub(super) const MAX_EXTERNAL_BYTES: u64 = 64 * 1024 * 1024;
const HEADER_BYTES: usize = HEADER_SECTORS * SECTOR_BYTES;

fn invalid_header() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid Anvil sector location")
}

fn pack_location(offset: usize, count: usize) -> io::Result<u32> {
    if !(HEADER_SECTORS..=MAX_SECTOR_OFFSET).contains(&offset)
        || !(1..=usize::from(u8::MAX)).contains(&count)
        || offset + count > MAX_SECTOR_OFFSET + 1
    {
        return Err(invalid_header());
    }
    Ok(((offset as u32) << u8::BITS) | count as u32)
}

pub(super) fn external_path(region: &Path, index: usize) -> io::Result<PathBuf> {
    let mut parts = region
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid_header)?
        .split('.');
    if parts.next() != Some("r") {
        return Err(invalid_header());
    }
    let region_x: i32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(invalid_header)?;
    let region_z: i32 = parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(invalid_header)?;
    let x = region_x
        .checked_mul(REGION_SIZE as i32)
        .and_then(|x| x.checked_add((index % REGION_SIZE) as i32))
        .ok_or_else(invalid_header)?;
    let z = region_z
        .checked_mul(REGION_SIZE as i32)
        .and_then(|z| z.checked_add((index / REGION_SIZE) as i32))
        .ok_or_else(invalid_header)?;
    Ok(region.with_file_name(format!("c.{x}.{z}.mcc")))
}

struct ExternalCommit {
    target: PathBuf,
    temporary: Option<TemporaryFile>,
}

impl ExternalCommit {
    async fn run(self) -> io::Result<()> {
        if let Some(temporary) = self.temporary {
            temporary.publish(&self.target).await?;
        } else {
            match tokio::fs::remove_file(&self.target).await {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        sync_parent_async(&self.target).await
    }
}

// RegionFile.usedSectors survives exceptions. Retain old and attempted allocations
// until the entry has been rewritten and forced, including on cancelled writes.
#[derive(Default)]
pub(super) struct RegionBitmap {
    pub(super) header: Vec<u8>,
    locations: Vec<Vec<(usize, usize)>>,
    used: Vec<bool>,
}

impl RegionBitmap {
    pub(super) fn initialize(&mut self, bytes: &[u8], length: u64) {
        if !self.header.is_empty() {
            return;
        }
        self.header = vec![0; HEADER_BYTES];
        let copied = bytes.len().min(HEADER_BYTES);
        self.header[..copied].copy_from_slice(&bytes[..copied]);
        // RegionFile.<init>: reserve every in-range entry, including overlapping pairs.
        // Only the starting sector must be within the file; missing padding is allowed.
        self.locations = self.header[..SECTOR_BYTES]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
            .map(|(index, entry)| {
                let location = u32::from_be_bytes(*entry);
                let offset = (location >> u8::BITS) as usize;
                let count = (location & u32::from(u8::MAX)) as usize;
                if location == 0 {
                    Vec::new()
                } else if offset < HEADER_SECTORS
                    || count == 0
                    || (offset * SECTOR_BYTES) as u64 > length
                {
                    tracing::warn!("Ignoring invalid Anvil location at index {index}");
                    *entry = [0; 4];
                    Vec::new()
                } else {
                    vec![(offset, count)]
                }
            })
            .collect();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.used = vec![true; HEADER_SECTORS];
        // Preserve sectors still referenced by another overlapping entry.
        for &(offset, count) in self.locations.iter().flatten() {
            let end = offset + count;
            self.used.resize(self.used.len().max(end), false);
            self.used[offset..end].fill(true);
        }
    }

    fn reserve(&mut self, index: usize, count: usize) -> io::Result<usize> {
        let offset = allocate(&mut self.used, count)?;
        self.locations[index].push((offset, count));
        Ok(offset)
    }

    fn published(&mut self, index: usize, offset: usize, count: usize) {
        self.locations[index] = vec![(offset, count)];
    }
}

fn allocate(used: &mut Vec<bool>, count: usize) -> io::Result<usize> {
    let mut free = 0;
    for offset in HEADER_SECTORS..used.len() {
        free = if used[offset] { 0 } else { free + 1 };
        if free == count {
            let start = offset + 1 - count;
            pack_location(start, count)?;
            used[start..start + count].fill(true);
            return Ok(start);
        }
    }
    let start = used.len() - free;
    pack_location(start, count)?;
    used.resize(start + count, true);
    used[start..].fill(true);
    Ok(start)
}

impl<S: SingleChunkDataSerializer> AnvilChunkFile<S> {
    async fn prepare_external(&self, path: &Path, index: usize) -> io::Result<ExternalCommit> {
        let target = external_path(path, index)?;
        let data = self.chunks_data[index]
            .as_ref()
            .ok_or_else(invalid_header)?;
        let temporary = if data.serialized_data.is_external() {
            if data.serialized_data.compressed_data.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "External chunk stream was not loaded",
                ));
            }
            if data.serialized_data.compressed_data.len() as u64 > MAX_EXTERNAL_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "External chunk stream exceeds read limit",
                ));
            }
            Some(
                TemporaryFile::write(&target, vec![data.serialized_data.compressed_data.clone()])
                    .await?,
            )
        } else {
            None
        };
        Ok(ExternalCommit { target, temporary })
    }

    pub(super) async fn write_indices<I>(&self, path: &Path, indices: I) -> io::Result<()>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .await?;
        let mut reservations = self.reservations.lock().await;
        if reservations.header.is_empty() {
            let length = file.metadata().await?.len();
            let mut bytes = vec![0; length.min(HEADER_BYTES as u64) as usize];
            file.read_exact(&mut bytes).await?;
            reservations.initialize(&bytes, length);
        }
        if file.metadata().await?.len() < HEADER_BYTES as u64 {
            file.set_len(HEADER_BYTES as u64).await?;
        }
        let mut header = reservations.header.clone();
        let mut publications = Vec::new();
        // RegionFile.write/RegionBitmap.free: retain old allocations until publication.
        // Batch each region flush: all payloads, force, all headers, force.
        for index in indices {
            if let Some(metadata) = &self.chunks_data[index] {
                let count = metadata.serialized_data.sector_count() as usize;
                let offset = reservations.reserve(index, count)?;
                let commit = self.prepare_external(path, index).await?;
                file.seek(std::io::SeekFrom::Start((offset * SECTOR_BYTES) as u64))
                    .await?;
                metadata.serialized_data.write(&mut file).await?;
                header[index * 4..index * 4 + 4]
                    .copy_from_slice(&pack_location(offset, count)?.to_be_bytes());
                let timestamp_offset = SECTOR_BYTES + index * 4;
                header[timestamp_offset..timestamp_offset + 4]
                    .copy_from_slice(&metadata.timestamp.to_be_bytes());
                publications.push((index, offset, count, commit));
            }
        }
        file.sync_all().await?;
        // Publish sidecars first so a failed rename cannot strand an inline record.
        let mut deletions = Vec::new();
        for (index, _, _, commit) in &mut publications {
            if let Some(temporary) = commit.temporary.take() {
                temporary.publish(&commit.target).await?;
            } else {
                deletions.push(external_path(path, *index)?);
            }
        }
        for (index, _, _, _) in &publications {
            self.publish_header(&mut file, &header, *index).await?;
        }
        self.sync_header(&file).await?;
        reservations.header = header;
        for (index, offset, count, _) in publications {
            reservations.published(index, offset, count);
        }
        reservations.rebuild();
        for target in deletions {
            ExternalCommit {
                target,
                temporary: None,
            }
            .run()
            .await?;
        }
        // Header publication is not atomic across a crash, as in RegionFile.writeHeader.
        sync_parent_async(path).await
    }

    // RegionFile.writeHeader: this four-byte location rewrite can tear on power loss.
    async fn publish_header(
        &self,
        file: &mut tokio::fs::File,
        header: &[u8],
        index: usize,
    ) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fail_before_header
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::Error::other(
                "Injected failure before header publication",
            ));
        }
        file.seek(std::io::SeekFrom::Start((index * 4) as u64))
            .await?;
        #[cfg(test)]
        if self
            .fail_header_write
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            file.write_all(&header[index * 4..index * 4 + 3]).await?;
            return Err(io::Error::other("Injected partial header write"));
        }
        file.write_all(&header[index * 4..index * 4 + 4]).await?;
        let timestamp_offset = SECTOR_BYTES + index * 4;
        file.seek(std::io::SeekFrom::Start(timestamp_offset as u64))
            .await?;
        file.write_all(&header[timestamp_offset..timestamp_offset + 4])
            .await?;
        Ok(())
    }

    async fn sync_header(&self, file: &tokio::fs::File) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fail_header_sync
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::Error::other("Injected header fsync failure"));
        }
        file.sync_all().await
    }

    #[cfg(test)]
    pub(super) async fn write_all(&self, path: &Path) -> io::Result<()> {
        self.write_indices(
            path,
            (0..super::CHUNK_COUNT).filter(|&i| self.chunks_data[i].is_some()),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_bounds_are_checked() {
        assert!(pack_location(2, 256).is_err());
        assert!(pack_location(MAX_SECTOR_OFFSET + 1, 1).is_err());
        assert!(pack_location(1, 1).is_err());
    }
}
