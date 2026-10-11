use super::*;
use crate::world_info::data_files::{
    DimensionClock, WorldClocksData, read_world_clocks, write_world_clocks,
};
use std::{cell::Cell, io::Write};

thread_local! {
    static FAIL_AFTER_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(super) struct InterruptibleWriter<'a> {
    file: &'a mut fs::File,
    remaining: Option<usize>,
}

impl<'a> InterruptibleWriter<'a> {
    pub(super) fn new(file: &'a mut fs::File) -> Self {
        Self {
            file,
            remaining: FAIL_AFTER_BYTES.get(),
        }
    }
}

impl Write for InterruptibleWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = match self.remaining {
            Some(0) => return Err(io::Error::other("Injected saved-data interruption")),
            Some(remaining) => bytes.len().min(remaining),
            None => bytes.len(),
        };
        let written = self.file.write(&bytes[..length])?;
        if let Some(remaining) = &mut self.remaining {
            *remaining -= written;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[test]
fn interrupted_clock_save_preserves_previous_document_and_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data/minecraft/world_clocks.dat");
    let mut clocks = WorldClocksData::default();
    clocks.clocks.insert(
        "minecraft:overworld".into(),
        DimensionClock {
            total_ticks: 6007,
            partial_tick: 0.5,
            rate: 2.5,
            paused: true,
        },
    );
    write_world_clocks(directory.path(), &clocks).unwrap();
    let previous = fs::read(&path).unwrap();
    let mut changed = clocks.clone();
    changed
        .clocks
        .get_mut("minecraft:overworld")
        .unwrap()
        .total_ticks = 9000;

    FAIL_AFTER_BYTES.set(Some(12));
    let result = write_world_clocks(directory.path(), &changed);
    FAIL_AFTER_BYTES.set(None);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Injected saved-data interruption")
    );
    assert_eq!(fs::read(&path).unwrap(), previous);
    assert_eq!(read_world_clocks(directory.path()), clocks);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);

    // A killed process can leave an incomplete sibling, which the reader must ignore.
    let abandoned = crate::temporary_path(&path);
    fs::write(&abandoned, [0x1f, 0x8b, 0x08]).unwrap();
    assert_eq!(read_world_clocks(directory.path()), clocks);
    write_world_clocks(directory.path(), &changed).unwrap();
    assert_eq!(read_world_clocks(directory.path()), changed);
    assert_eq!(fs::read(abandoned).unwrap(), [0x1f, 0x8b, 0x08]);
}
