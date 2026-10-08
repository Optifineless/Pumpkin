use super::{RandomSequence, RandomSequences, Xoroshiro};
use pumpkin_nbt::{Nbt, compound::NbtCompound, deserializer::NbtReadHelperJava, tag::NbtTag};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};
const MAX_SAVED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SAVED_SEQUENCES: usize = 65536;
const SAVED_PATH: &str = "data/minecraft/random_sequences.dat";

impl RandomSequences {
    /// Load vanilla 26.3 `SavedDataStorage`'s namespaced random-sequence document.
    pub fn load(world: &Path) -> io::Result<Self> {
        let path = world.join(SAVED_PATH);
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() > MAX_SAVED_BYTES {
            return Err(io::Error::other("Random sequences file is too large"));
        }
        let mut input = io::BufReader::new(file);
        let mut magic = [0; 2];
        input.read_exact(&mut magic)?;
        let input = io::Cursor::new(magic).chain(input);
        let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] {
            Box::new(flate2::read::GzDecoder::new(input))
        } else {
            Box::new(input)
        };
        let mut bytes = Vec::new();
        reader.take(MAX_SAVED_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SAVED_BYTES {
            return Err(io::Error::other("Random sequences data is too large"));
        }
        let nbt = Nbt::read(&mut NbtReadHelperJava::new(&mut io::Cursor::new(bytes)))
            .map_err(io::Error::other)?;
        Self::from_nbt(&nbt.root_tag)
            .ok_or_else(|| io::Error::other("Invalid random sequences data"))
    }

    fn from_nbt(root: &NbtCompound) -> Option<Self> {
        // RandomSequences.CODEC -> RandomSequence.CODEC -> Xoroshiro128PlusPlus.CODEC.
        let data = root.get_compound("data")?;
        let sequences = data.get_compound("sequences")?;
        if sequences.child_tags.len() > MAX_SAVED_SEQUENCES {
            return None;
        }
        let mut result = Self::new();
        result.salt = data.get_int("salt")?;
        result.include_world_seed = data.get_bool("include_world_seed").unwrap_or(true);
        result.include_sequence_id = data.get_bool("include_sequence_id").unwrap_or(true);
        for (name, sequence) in &sequences.child_tags {
            let id = pumpkin_util::identifier::Identifier::parse(name).ok()?;
            let [lo, hi] = sequence.extract_compound()?.get_long_array("source")? else {
                return None;
            };
            result.sequences.insert(
                id.to_string(),
                RandomSequence {
                    rng: Xoroshiro::new(*lo as u64, *hi as u64),
                },
            );
        }
        Some(result)
    }

    fn to_nbt(&self, data_version: i32) -> NbtCompound {
        let mut data = NbtCompound::new();
        data.put_int("salt", self.salt);
        data.put_bool("include_world_seed", self.include_world_seed);
        data.put_bool("include_sequence_id", self.include_sequence_id);
        let mut sequences = NbtCompound::new();
        for (name, sequence) in &self.sequences {
            let mut value = NbtCompound::new();
            value.put("source", NbtTag::LongArray(sequence.rng.state().to_vec()));
            sequences.put_compound(name, value);
        }
        data.put_compound("sequences", sequences);
        let mut root = NbtCompound::new();
        root.put_compound("data", data);
        root.put_int("DataVersion", data_version);
        root
    }

    /// Save current words and defaults; retain dirty state if writing fails.
    pub fn save(&mut self, world: &Path, data_version: i32) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let path = world.join(SAVED_PATH);
        fs::create_dir_all(world.join("data/minecraft"))?;
        let temporary = path.with_extension("dat.tmp");
        let file = fs::File::create(&temporary)?;
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(&Nbt::new(String::new(), self.to_nbt(data_version)).write())?;
        encoder.finish()?.sync_all()?;
        fs::rename(temporary, path)?;
        self.dirty = false;
        Ok(())
    }
}
