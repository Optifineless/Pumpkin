use std::{
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom},
    path::Path,
};

use flate2::read::GzDecoder;
use pumpkin_nbt::{
    Nbt, NbtCompound,
    deserializer::{NbtReadHelper, NbtReadHelperJava, NbtStreamReader},
};

use super::{
    WorldInfoError,
    data_files::{ensure_minecraft_data_dir, minecraft_data_dir, write_saved_data},
};

#[cfg(test)]
mod tests;

const MAX_SCOREBOARD_BYTES: u64 = 64 * 1024 * 1024;

/// Reads the complete saved-data document without interpreting scoreboard fields.
/// Callers must disable saves after any load or codec error to preserve the original file.
pub fn read_scoreboard(level_folder: &Path) -> Result<Option<NbtCompound>, WorldInfoError> {
    let path = minecraft_data_dir(level_folder).join("scoreboard.dat");
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // DimensionStorageFileFix (26.1) moved this saved data into the namespace.
            // Import without moving the original: structural decoding can still fail later.
            match File::open(level_folder.join("data/scoreboard.dat")) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_SCOREBOARD_BYTES {
        return Err(invalid_data(
            "scoreboard.dat exceeds the compressed size limit",
        ));
    }
    let mut bytes = Vec::new();
    GzDecoder::new(file.take(MAX_SCOREBOARD_BYTES + 1))
        .take(MAX_SCOREBOARD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SCOREBOARD_BYTES {
        return Err(invalid_data(
            "scoreboard.dat exceeds the decompressed size limit",
        ));
    }

    // Other NBT readers accept EOF as a compound terminator. A saved scoreboard
    // must be complete before it can safely replace the player's document later.
    let input = StrictInput(Cursor::new(bytes.as_slice()));
    let mut reader = NbtReadHelperJava::new(NbtStreamReader(input));
    let root = Nbt::read(&mut reader)
        .map_err(|error| invalid_data(&error.to_string()))?
        .root_tag;
    if reader.reader().0.0.position() != bytes.len() as u64 {
        return Err(invalid_data("Trailing bytes in scoreboard.dat NBT"));
    }
    if root.get_compound("data").is_none() {
        return Err(invalid_data("scoreboard.dat is missing its data compound"));
    }
    Ok(Some(root))
}

/// Atomically writes a `ScoreboardSaveData.Packed` payload using the server's data version.
/// Callers must serialize writes and must not call this after a failed load.
pub fn write_scoreboard(
    level_folder: &Path,
    data: NbtCompound,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let directory = ensure_minecraft_data_dir(level_folder)?;
    let mut root = NbtCompound::new();
    // SavedDataStorage.tryWrite wraps ScoreboardSaveData.Packed.CODEC in data/DataVersion.
    root.put_int("DataVersion", data_version);
    root.put_compound("data", data);
    write_saved_data(&directory.join("scoreboard.dat"), root)?;
    Ok(())
}

fn invalid_data(message: &str) -> WorldInfoError {
    WorldInfoError::DeserializationError(message.to_owned())
}

struct StrictInput<'a>(Cursor<&'a [u8]>);

impl Read for StrictInput<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.0.read(buffer)?;
        if count == 0 && !buffer.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing NBT compound terminator in scoreboard.dat",
            ));
        }
        Ok(count)
    }
}

impl Seek for StrictInput<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}
