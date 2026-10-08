use super::{
    ReadingError,
    nbt_budget::{DEFAULT_NBT_QUOTA, MAX_STACK_DEPTH},
};
use pumpkin_nbt::{compound::NbtCompound, deserializer::NbtReadHelper, tag::NbtTag};

pub(super) struct BudgetedNbtReader {
    usage: usize,
}

impl BudgetedNbtReader {
    pub(super) const fn new() -> Self {
        Self { usage: 0 }
    }

    fn account(&mut self, count: usize) -> Result<(), ReadingError> {
        self.usage = self
            .usage
            .checked_add(count)
            .filter(|usage| *usage <= DEFAULT_NBT_QUOTA)
            .ok_or_else(|| ReadingError::TooLarge("NBT quota".into()))?;
        Ok(())
    }

    pub(super) fn string<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        overhead: usize,
    ) -> Result<String, ReadingError> {
        let value = read.get_string().map_err(|error| nbt_error(&error))?;
        self.account(overhead + 2 * value.encode_utf16().count())?;
        Ok(value.into_owned())
    }

    fn length<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        overhead: usize,
        width: usize,
    ) -> Result<usize, ReadingError> {
        let count = usize::try_from(read.get_i32().map_err(|error| nbt_error(&error))?)
            .map_err(|_| ReadingError::Message("Negative NBT length".into()))?;
        self.account(overhead)?;
        self.account(
            count
                .checked_mul(width)
                .ok_or_else(|| ReadingError::TooLarge("NBT array".into()))?,
        )?;
        Ok(count)
    }

    pub(super) fn tag<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        id: u8,
        depth: usize,
    ) -> Result<NbtTag, ReadingError> {
        // NbtAccounter.pushDepth: count only list/compound containers, once per container.
        if matches!(id, 9 | 10) && depth >= MAX_STACK_DEPTH {
            return Err(ReadingError::TooLarge("NBT depth".into()));
        }
        // TagType.load dispatch: keep scalar temporaries off the recursive container stack.
        match id {
            9 => self.list(read, depth + 1),
            10 => self.compound(read, depth + 1),
            _ => self.non_container_tag(read, id),
        }
    }

    fn non_container_tag<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        id: u8,
    ) -> Result<NbtTag, ReadingError> {
        let tag = match id {
            0 => {
                self.account(8)?;
                NbtTag::End
            }
            1 => {
                self.account(9)?;
                NbtTag::Byte(read.get_i8().map_err(|error| nbt_error(&error))?)
            }
            2 => {
                self.account(10)?;
                NbtTag::Short(read.get_i16().map_err(|error| nbt_error(&error))?)
            }
            3 => {
                self.account(12)?;
                NbtTag::Int(read.get_i32().map_err(|error| nbt_error(&error))?)
            }
            4 => {
                self.account(16)?;
                NbtTag::Long(read.get_i64().map_err(|error| nbt_error(&error))?)
            }
            5 => {
                self.account(12)?;
                NbtTag::Float(read.get_f32().map_err(|error| nbt_error(&error))?)
            }
            6 => {
                self.account(16)?;
                NbtTag::Double(read.get_f64().map_err(|error| nbt_error(&error))?)
            }
            7 => {
                let count = self.length(read, 24, 1)?;
                NbtTag::ByteArray(
                    read.get_byte_array(count)
                        .map_err(|error| nbt_error(&error))?
                        .into_owned()
                        .into(),
                )
            }
            8 => NbtTag::String(self.string(read, 36)?.into()),
            11 => {
                let count = self.length(read, 24, 4)?;
                NbtTag::IntArray(
                    read.get_i32_array(count)
                        .map_err(|error| nbt_error(&error))?,
                )
            }
            12 => {
                let count = self.length(read, 24, 8)?;
                NbtTag::LongArray(
                    read.get_i64_array(count)
                        .map_err(|error| nbt_error(&error))?,
                )
            }
            _ => return Err(ReadingError::Message("Unknown NBT tag".into())),
        };
        Ok(tag)
    }

    fn list<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        depth: usize,
    ) -> Result<NbtTag, ReadingError> {
        let id = read.get_u8().map_err(|error| nbt_error(&error))?;
        let count = self.length(read, 36, 4)?;
        if id == 0 && count != 0 {
            return Err(ReadingError::Message("Nonempty end-tag list".into()));
        }
        let mut list = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let mut tag = self.tag(read, id, depth)?;
            // ListTag.addAndUnwrap preserves the wrapper convention for heterogeneous lists.
            if let NbtTag::Compound(compound) = &mut tag
                && compound.child_tags.len() == 1
                && compound.child_tags.contains_key("")
                && let Some(value) = compound.child_tags.remove("")
            {
                tag = value;
            }
            list.push(tag);
        }
        Ok(NbtTag::List(list))
    }

    fn compound<'a>(
        &mut self,
        read: &mut impl NbtReadHelper<'a>,
        depth: usize,
    ) -> Result<NbtTag, ReadingError> {
        self.account(48)?;
        let mut compound = NbtCompound::new();
        loop {
            let id = read.get_u8().map_err(|error| nbt_error(&error))?;
            if id == 0 {
                break;
            }
            let key = self.string(read, 28)?;
            let value = self.tag(read, id, depth)?;
            if compound.child_tags.insert(key.into(), value).is_none() {
                self.account(36)?;
            }
        }
        Ok(NbtTag::Compound(compound))
    }
}

fn nbt_error(error: &pumpkin_nbt::Error) -> ReadingError {
    ReadingError::Message(error.to_string())
}

/// Reads named network NBT with the Java `NbtAccounter` quota and container-depth rules.
/// The helper's byte source must bound string reads by the enclosing payload length.
pub fn read_named_nbt<'a>(
    read: &mut impl NbtReadHelper<'a>,
) -> Result<pumpkin_nbt::Nbt, ReadingError> {
    let mut parser = BudgetedNbtReader::new();
    let id = read.get_u8().map_err(|error| nbt_error(&error))?;
    if id == 0 {
        return Ok(pumpkin_nbt::Nbt::default());
    }
    // NbtIo.readUnnamedTag excludes the root name from tag heap accounting.
    let name = read
        .get_string()
        .map_err(|error| nbt_error(&error))?
        .into_owned();
    match parser.tag(read, id, 0)? {
        NbtTag::Compound(root) => Ok(pumpkin_nbt::Nbt::new(name, root)),
        _ => Err(ReadingError::Message("NBT root must be a compound".into())),
    }
}
