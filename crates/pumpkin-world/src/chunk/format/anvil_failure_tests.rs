use super::storage_tests::{RawChunk, put};
use super::*;
use std::sync::atomic::Ordering;

async fn fetch(file: &AnvilChunkFile<RawChunk>, x: i32) -> LoadedData<RawChunk, ChunkReadingError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    file.get_chunks(
        vec![Vector2::new(x, 0)],
        tx,
        pumpkin_data::dimension::Dimension::OVERWORLD,
    )
    .await;
    rx.recv().await.unwrap()
}

#[tokio::test]
async fn failed_external_publication_preserves_inline_record_and_retries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let sidecar = directory.path().join("c.0.0.mcc");
    let mut file = AnvilChunkFile::<RawChunk>::default();
    put(&mut file, 0, Bytes::from_static(b"old"));
    file.write_indices(&path, [0]).await.unwrap();
    let original = std::fs::read(&path).unwrap();
    let inode = directory.path().join("original-inode");
    std::fs::hard_link(&path, &inode).unwrap();
    put(&mut file, 0, Bytes::from(vec![0x55; 255 * SECTOR_BYTES]));
    std::fs::create_dir(&sidecar).unwrap();
    assert!(file.write_indices(&path, [0]).await.is_err());
    assert_eq!(&std::fs::read(&path).unwrap()[..original.len()], original);
    assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")
    }));
    std::fs::remove_dir(&sidecar).unwrap();
    // Failure after the sidecar rename but before the location entry is changed.
    file.fail_before_header.store(true, Ordering::Relaxed);
    assert!(file.write_indices(&path, [0]).await.is_err());
    let interrupted = std::fs::read(&path).unwrap();
    assert_eq!(&interrupted[..original.len()], original);
    let reopened = AnvilChunkFile::<RawChunk>::read_with_path(interrupted.into(), &path).unwrap();
    let LoadedData::Loaded(old) = fetch(&reopened, 0).await else {
        panic!("old inline record lost")
    };
    assert_eq!(old.0, b"old"[..]);
    file.write_indices(&path, [0]).await.unwrap();
    let committed = std::fs::read(&path).unwrap();
    assert_eq!(std::fs::read(inode).unwrap(), committed); // No replacement of the region inode.
    assert_eq!(
        committed[2 * SECTOR_BYTES + 5..2 * SECTOR_BYTES + 8],
        *b"old"
    );
    let reopened = AnvilChunkFile::<RawChunk>::read_with_path(committed.into(), &path).unwrap();
    let LoadedData::Loaded(new) = fetch(&reopened, 0).await else {
        panic!("retry failed")
    };
    assert_eq!(new.0.len(), 255 * SECTOR_BYTES);
}

#[tokio::test]
async fn missing_sidecar_and_truncated_payload_are_isolated() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    // RegionFile framing, handwritten: external stub, healthy inline record, truncated record.
    let mut bytes = vec![0; 4 * SECTOR_BYTES + 7];
    bytes[..12].copy_from_slice(&[0, 0, 2, 1, 0, 0, 3, 1, 0, 0, 4, 1]);
    bytes[2 * SECTOR_BYTES..2 * SECTOR_BYTES + 5].copy_from_slice(&[0, 0, 0, 1, 0x82]);
    bytes[3 * SECTOR_BYTES..3 * SECTOR_BYTES + 8]
        .copy_from_slice(&[0, 0, 0, 4, 3, b'o', b'k', b'!']);
    bytes[4 * SECTOR_BYTES..].copy_from_slice(&[0, 0, 0, 4, 3, b'n', b'o']);
    let file = AnvilChunkFile::<RawChunk>::read_with_path(bytes.clone().into(), &path).unwrap();
    assert!(matches!(fetch(&file, 0).await, LoadedData::Error(_)));
    let LoadedData::Loaded(healthy) = fetch(&file, 1).await else {
        panic!("healthy neighbour rejected")
    };
    assert_eq!(healthy.0, b"ok!"[..]);
    assert!(matches!(fetch(&file, 2).await, LoadedData::Error(_)));
    bytes.push(b'w'); // Completing the declared payload is sufficient; no sector padding required.
    let file = AnvilChunkFile::<RawChunk>::read(bytes.into()).unwrap();
    let LoadedData::Loaded(complete) = fetch(&file, 2).await else {
        panic!("unpadded record rejected")
    };
    assert_eq!(complete.0, b"now"[..]);
}

#[tokio::test]
async fn truncated_header_is_zero_padded_and_writable() {
    let mut bytes = vec![0; 8192];
    bytes[4..8].copy_from_slice(&[0, 0, 2, 1]);
    bytes.truncate(6); // A torn location entry, between its high and low bytes.
    bytes[4..6].copy_from_slice(&[0, 1]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    std::fs::write(&path, &bytes).unwrap();
    let mut file = AnvilChunkFile::<RawChunk>::read(bytes.into()).unwrap();
    for x in 0..3 {
        assert!(matches!(fetch(&file, x).await, LoadedData::Missing(_)));
    }
    put(&mut file, 2, Bytes::from_static(b"saved"));
    file.write_indices(&path, [2]).await.unwrap();
    let reopened = AnvilChunkFile::<RawChunk>::read(std::fs::read(&path).unwrap().into()).unwrap();
    assert!(matches!(fetch(&reopened, 2).await, LoadedData::Loaded(_)));
}

#[tokio::test]
async fn reads_compressed_external_format_fixtures() {
    // RegionFileVersion IDs 1/2/4: independently encoded gzip/zlib/Java LZ4 streams.
    // NBT is a named root with a 256-byte "data" byte array, followed by TAG_End.
    // LZ4BlockOutputStream framing and XXHash32's Java checksum mask; bytes produced
    // with system liblz4/libxxhash, not Pumpkin's compressor.
    let fixtures: &[(u8, &[u8])] = &[
        (
            1,
            &[
                0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xe3, 0x62, 0x60, 0x60,
                0x67, 0x60, 0x49, 0x49, 0x2c, 0x49, 0x64, 0x60, 0x60, 0x64, 0x70, 0x1c, 0xe1, 0x80,
                0x01, 0x00, 0x03, 0x81, 0xb0, 0x9b, 0x0f, 0x01, 0x00, 0x00,
            ],
        ),
        (
            2,
            &[
                0x78, 0x9c, 0xe3, 0x62, 0x60, 0x60, 0x67, 0x60, 0x49, 0x49, 0x2c, 0x49, 0x64, 0x60,
                0x60, 0x64, 0x70, 0x1c, 0xe1, 0x80, 0x01, 0x00, 0xa1, 0xa0, 0x42, 0xb1,
            ],
        ),
        (
            4,
            &[
                0x4c, 0x5a, 0x34, 0x42, 0x6c, 0x6f, 0x63, 0x6b, 0x26, 0x1a, 0x00, 0x00, 0x00, 0x0f,
                0x01, 0x00, 0x00, 0xd9, 0xdd, 0x61, 0x0c, 0xff, 0x00, 0x0a, 0x00, 0x00, 0x07, 0x00,
                0x04, 0x64, 0x61, 0x74, 0x61, 0x00, 0x00, 0x01, 0x00, 0x41, 0x01, 0x00, 0xe8, 0x50,
                0x41, 0x41, 0x41, 0x41, 0x00, 0x4c, 0x5a, 0x34, 0x42, 0x6c, 0x6f, 0x63, 0x6b, 0x16,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        ),
    ];
    let mut expected = b"\x0a\0\0\x07\0\x04data\0\0\x01\0".to_vec();
    expected.extend_from_slice(&[b'A'; 256]);
    expected.push(0);
    for (id, compressed) in fixtures {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("r.0.0.mca");
        let mut region = vec![0; 8197];
        region[..4].copy_from_slice(&[0, 0, 2, 1]);
        region[8192..].copy_from_slice(&[0, 0, 0, 1, 0x80 | id]);
        std::fs::write(directory.path().join("c.0.0.mcc"), compressed).unwrap();
        let file = AnvilChunkFile::<RawChunk>::read_with_path(region.into(), &path).unwrap();
        let LoadedData::Loaded(chunk) = fetch(&file, 0).await else {
            panic!("external compression ID {id} failed")
        };
        assert_eq!(chunk.0.as_ref(), expected);
    }
}

#[tokio::test]
async fn interrupted_external_replacement_keeps_uncompressed_stub_readable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut region = vec![0; 8197];
    region[..4].copy_from_slice(&[0, 0, 2, 1]);
    region[8192..].copy_from_slice(&[0, 0, 0, 1, 0x83]);
    std::fs::write(&path, &region).unwrap();
    std::fs::write(directory.path().join("c.0.0.mcc"), b"old raw stream").unwrap();
    let mut file = AnvilChunkFile::<RawChunk>::read_with_path(region.into(), &path).unwrap();
    let payload = Bytes::from(vec![0x56; 255 * SECTOR_BYTES]);
    file.update_chunk(
        Arc::new(RawChunk(payload.clone())),
        &AnvilChunkConfig::default(),
    )
    .await
    .unwrap();
    file.fail_before_header.store(true, Ordering::Relaxed);
    assert!(file.write(&path).await.is_err());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes[8196], 0x83); // Still the previous uncompressed external stub.
    let reopened = AnvilChunkFile::<RawChunk>::read_with_path(bytes.into(), &path).unwrap();
    let LoadedData::Loaded(chunk) = fetch(&reopened, 0).await else {
        panic!("old stub cannot read published sidecar")
    };
    assert_eq!(chunk.0, payload);
    file.write(&path).await.unwrap();
}

#[tokio::test]
async fn header_write_failure_reserves_sectors_through_retry() {
    uncertain_header_allocations_survive_retry(false).await;
}

#[tokio::test]
async fn header_fsync_failure_reserves_sectors_through_retry() {
    uncertain_header_allocations_survive_retry(true).await;
}

async fn uncertain_header_allocations_survive_retry(fail_sync: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut file = AnvilChunkFile::<RawChunk>::default();
    put(&mut file, 0, Bytes::from_static(b"old"));
    put(&mut file, 1, Bytes::from_static(b"neighbor"));
    file.write_indices(&path, [0, 1]).await.unwrap();
    let original = std::fs::read(&path).unwrap();
    put(&mut file, 0, Bytes::from_static(b"new"));
    let failure = if fail_sync {
        &file.fail_header_sync
    } else {
        &file.fail_header_write
    };
    failure.store(true, Ordering::Relaxed);
    assert!(file.write_indices(&path, [0]).await.is_err());
    let failed = std::fs::read(&path).unwrap();
    put(&mut file, 0, Bytes::from_static(b"retry"));
    // Retry must not reuse sector 2 even if page cache now advertises sector 4.
    file.fail_before_header.store(true, Ordering::Relaxed);
    assert!(file.write_indices(&path, [0]).await.is_err());
    let retry = std::fs::read(&path).unwrap();
    assert_eq!(
        &retry[2 * SECTOR_BYTES..3 * SECTOR_BYTES],
        &original[2 * SECTOR_BYTES..3 * SECTOR_BYTES]
    );
    assert_eq!(
        &retry[4 * SECTOR_BYTES..5 * SECTOR_BYTES],
        &failed[4 * SECTOR_BYTES..5 * SECTOR_BYTES]
    );
    file.write_indices(&path, [0]).await.unwrap();
    let reopened =
        AnvilChunkFile::<RawChunk>::read_with_path(std::fs::read(&path).unwrap().into(), &path)
            .unwrap();
    let LoadedData::Loaded(new) = fetch(&reopened, 0).await else {
        panic!("retry lost chunk")
    };
    let LoadedData::Loaded(neighbor) = fetch(&reopened, 1).await else {
        panic!("neighbor lost")
    };
    assert_eq!(new.0, b"retry"[..]);
    assert_eq!(neighbor.0, b"neighbor"[..]);
}

#[tokio::test]
async fn malformed_locations_do_not_block_healthy_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut bytes = vec![0; 3 * SECTOR_BYTES];
    bytes[..24].copy_from_slice(&[
        0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 8, 1, 0, 0, 2, 0,
    ]);
    bytes[8192..8200].copy_from_slice(&[0, 0, 0, 4, 3, b'o', b'l', b'd']);
    std::fs::write(&path, &bytes).unwrap();
    let mut file = AnvilChunkFile::<RawChunk>::read_with_path(bytes.into(), &path).unwrap();
    assert!(matches!(fetch(&file, 4).await, LoadedData::Missing(_)));
    assert!(matches!(fetch(&file, 5).await, LoadedData::Missing(_)));
    put(&mut file, 3, Bytes::from_static(b"healthy"));
    file.write_indices(&path, [3]).await.unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[12..16], &[0, 0, 4, 1]);
    put(&mut file, 1, Bytes::from_static(b"replaced overlap"));
    file.write_indices(&path, [1]).await.unwrap();
    put(&mut file, 3, Bytes::from_static(b"healthy"));
    file.write_indices(&path, [3]).await.unwrap();
    let reopened =
        AnvilChunkFile::<RawChunk>::read_with_path(std::fs::read(&path).unwrap().into(), &path)
            .unwrap();
    assert!(matches!(fetch(&reopened, 0).await, LoadedData::Missing(_)));
    let LoadedData::Loaded(healthy) = fetch(&reopened, 3).await else {
        panic!("healthy write lost")
    };
    let LoadedData::Loaded(overlap) = fetch(&reopened, 2).await else {
        panic!("overlap overwritten")
    };
    assert_eq!(healthy.0, b"healthy"[..]);
    assert_eq!(overlap.0, b"old"[..]);
}

#[tokio::test]
async fn region_batch_writes_every_payload_before_publishing_headers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("r.0.0.mca");
    let mut file = AnvilChunkFile::<RawChunk>::default();
    put(&mut file, 0, Bytes::from_static(b"old first"));
    put(&mut file, 1, Bytes::from_static(b"old second"));
    file.write_indices(&path, [0, 1]).await.unwrap();
    let before = std::fs::read(&path).unwrap();
    put(&mut file, 0, Bytes::from_static(b"new first"));
    put(&mut file, 1, Bytes::from_static(b"new second"));
    file.fail_before_header.store(true, Ordering::Relaxed);
    assert!(file.write_indices(&path, [0, 1]).await.is_err());
    let after = std::fs::read(&path).unwrap();
    assert_eq!(&after[..before.len()], before);
    assert!(
        after
            .windows(b"new first".len())
            .any(|bytes| bytes == b"new first")
    );
    assert!(
        after
            .windows(b"new second".len())
            .any(|bytes| bytes == b"new second")
    );
    file.write_indices(&path, [0, 1]).await.unwrap();
    let reopened = AnvilChunkFile::<RawChunk>::read(std::fs::read(path).unwrap().into()).unwrap();
    let LoadedData::Loaded(first) = fetch(&reopened, 0).await else {
        panic!("first chunk lost")
    };
    let LoadedData::Loaded(second) = fetch(&reopened, 1).await else {
        panic!("second chunk lost")
    };
    assert_eq!(first.0, b"new first"[..]);
    assert_eq!(second.0, b"new second"[..]);
}
