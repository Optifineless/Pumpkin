use pumpkin_nbt::{compound::NbtCompound, nbt_compress::write_gzip_compound_tag};
use std::{fs, io, path::Path};

#[cfg(test)]
mod tests;
#[cfg(test)]
pub use tests::interrupt_file;

/// Publishes gzip saved data only after the complete temporary file has been forced to disk.
/// The parent directory must exist, and callers must serialize saves of the same state.
pub fn write_saved_data(path: &Path, root: NbtCompound) -> io::Result<()> {
    // SavedDataStorage.tryWrite encodes saved data; the fork forces a temporary before
    // replacement so interrupted clock/RNG autosaves preserve the previous document.
    let temporary = crate::temporary_path(path);
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        #[cfg(test)]
        let output = tests::InterruptibleWriter::new(&mut file, path);
        #[cfg(not(test))]
        let output = &mut file;
        write_gzip_compound_tag(root, output).map_err(io::Error::other)?;
        file.sync_all()?;
        drop(file);
        crate::replace(&temporary, path)?;
        crate::sync_parent(path)
    })();
    if result.is_err()
        && let Err(error) = fs::remove_file(&temporary)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("Failed to remove saved-data temporary: {error}");
    }
    result
}
