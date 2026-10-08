use super::*;

pub(super) struct RawChunk(pub(super) Bytes);

impl Dirtiable for RawChunk {
    fn is_dirty(&self) -> bool {
        true
    }
    fn mark_dirty(&self, _: bool) {}
}

impl SingleChunkDataSerializer for RawChunk {
    fn to_bytes(&self) -> Result<Bytes, ChunkSerializingError> {
        Ok(self.0.clone())
    }
    fn from_bytes(bytes: &Bytes, _: Vector2<i32>) -> Result<Self, ChunkReadingError> {
        Ok(Self(bytes.clone()))
    }
    fn position(&self) -> (i32, i32) {
        (0, 0)
    }
}

pub(super) fn put(file: &mut AnvilChunkFile<RawChunk>, index: usize, bytes: Bytes) {
    file.chunks_data[index] = Some(AnvilChunkMetadata {
        serialized_data: AnvilChunkData {
            compression: None,
            compressed_data: bytes,
            external: false,
            external_path: None,
        },
        timestamp: 1,
    });
}

#[tokio::test]
async fn oversized_roundtrip_and_inline_replacement_remove_external_stream() {
    for in_place in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("r.0.0.mca");
        let external = directory.path().join("c.0.0.mcc");
        // Five bytes of stream header make this require exactly 256 sectors.
        let payload = Bytes::from(vec![0x5a; 255 * SECTOR_BYTES - 4]);
        let mut file = AnvilChunkFile::<RawChunk>::default();
        put(&mut file, 0, payload.clone());
        if in_place {
            file.write_indices(&path, [0]).await.unwrap();
        } else {
            file.write_all(&path).await.unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 2, 1]);
        assert_eq!(
            &bytes[2 * SECTOR_BYTES..2 * SECTOR_BYTES + 5],
            &[0, 0, 0, 1, 0x83]
        );
        assert_eq!(std::fs::read(&external).unwrap().len(), payload.len());
        let read = AnvilChunkFile::<RawChunk>::read_with_path(bytes.into(), &path).unwrap();
        assert_eq!(
            read.chunks_data[0]
                .as_ref()
                .unwrap()
                .serialized_data
                .to_chunk::<RawChunk>(Vector2::new(0, 0))
                .unwrap()
                .0,
            payload
        );
        put(&mut file, 0, Bytes::from_static(b"small again"));
        if in_place {
            file.write_indices(&path, [0]).await.unwrap();
        } else {
            file.write_all(&path).await.unwrap();
        }
        assert!(!external.exists());
        let read =
            AnvilChunkFile::<RawChunk>::read_with_path(std::fs::read(&path).unwrap().into(), &path)
                .unwrap();
        assert_eq!(
            read.chunks_data[0]
                .as_ref()
                .unwrap()
                .serialized_data
                .compressed_data,
            b"small again"[..]
        );
    }
}

#[test]
fn reads_vanilla_style_external_stub_at_negative_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.-1.2.mca");
    // RegionFile.createExternalStub with uncompressed stream ID 3.
    // RegionFile.write writes only the five-byte stub; padding happens on close.
    let mut region = vec![0; 2 * SECTOR_BYTES + 5];
    region[..4].copy_from_slice(&[0, 0, 2, 1]);
    region[2 * SECTOR_BYTES..2 * SECTOR_BYTES + 5].copy_from_slice(&[0, 0, 0, 1, 0x83]);
    std::fs::write(
        directory.path().join("c.-32.64.mcc"),
        b"vanilla external payload",
    )
    .unwrap();
    let file = AnvilChunkFile::<RawChunk>::read_with_path(region.into(), &path).unwrap();
    let data = file.chunks_data[0]
        .as_ref()
        .unwrap()
        .serialized_data
        .to_chunk::<RawChunk>(Vector2::new(-32, 64))
        .unwrap();
    assert_eq!(data.0, b"vanilla external payload"[..]);
}

#[tokio::test]
async fn in_place_update_allocates_new_sectors_and_keeps_neighbours() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut file = AnvilChunkFile::<RawChunk>::default();
    put(&mut file, 0, Bytes::from_static(b"original"));
    put(&mut file, 1, Bytes::from_static(b"neighbour"));
    file.write_all(&path).await.unwrap();
    let old = std::fs::read(&path).unwrap();
    put(&mut file, 0, Bytes::from_static(b"updated"));
    file.write_indices(&path, [0]).await.unwrap();
    let new = std::fs::read(&path).unwrap();
    assert_ne!(&old[..4], &new[..4]);
    assert_eq!(&old[4..8], &new[4..8]);
    assert_eq!(&old[2 * SECTOR_BYTES..], &new[2 * SECTOR_BYTES..old.len()]);
    put(&mut file, 0, Bytes::from(vec![0xaa; 2 * SECTOR_BYTES]));
    file.write_indices(&path, [0]).await.unwrap();
    let grown = std::fs::read(&path).unwrap();
    assert_eq!(&old[4..8], &grown[4..8]);
    assert_eq!(
        &old[3 * SECTOR_BYTES..4 * SECTOR_BYTES],
        &grown[3 * SECTOR_BYTES..4 * SECTOR_BYTES]
    );
    let read = AnvilChunkFile::<RawChunk>::read(grown.into()).unwrap();
    assert_eq!(
        read.chunks_data[0]
            .as_ref()
            .unwrap()
            .serialized_data
            .compressed_data
            .len(),
        2 * SECTOR_BYTES
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn payload_write_failure_keeps_committed_header_and_original_chunks() {
    use std::process::Command;
    const TEST_NAME: &str = "chunk::format::anvil::storage_tests::payload_write_failure_keeps_committed_header_and_original_chunks";
    const TEST_PATH: &str = "PUMPKIN_TEST_FAILED_REGION_WRITE";
    if let Some(path) = std::env::var_os(TEST_PATH) {
        let path = PathBuf::from(path);
        let limit = libc::rlimit {
            rlim_cur: (4 * SECTOR_BYTES) as _,
            rlim_max: (4 * SECTOR_BYTES) as _,
        };
        // SAFETY: only this isolated test subprocess is affected; limit points to a valid rlimit.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &raw const limit) };
        assert_eq!(result, 0);
        // SAFETY: SIG_IGN is a valid disposition; EFBIG is returned instead of killing this child.
        unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) };
        let mut file = AnvilChunkFile::<RawChunk>::default();
        put(&mut file, 0, Bytes::from_static(b"interrupted update"));
        file.chunks_data[0].as_mut().unwrap().timestamp = 2;
        assert!(file.write_indices(&path, [0]).await.is_err());
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut file = AnvilChunkFile::<RawChunk>::default();
    put(&mut file, 0, Bytes::from_static(b"original"));
    put(&mut file, 1, Bytes::from_static(b"neighbour"));
    file.write_all(&path).await.unwrap();
    let original = std::fs::read(&path).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(TEST_PATH, &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
