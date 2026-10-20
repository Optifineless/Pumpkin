//! Saved scoreboard depth follows `NbtAccounter`, independently of other NBT readers.

use pumpkin_nbt::{
    COMPOUND_ID, END_ID, Error, LIST_ID, MAX_ARRAY_LENGTH, MAX_NBT_DEPTH, NbtCompound,
    deserializer::NbtReadHelper, tag::NbtTag,
};

pub(super) fn read<'a>(reader: &mut impl NbtReadHelper<'a>) -> Result<NbtCompound, Error> {
    let id = reader.get_u8()?;
    if id != COMPOUND_ID {
        return Err(Error::NoRootCompound(id));
    }
    reader.get_string()?;
    compound(reader, 1)
}

fn compound<'a>(reader: &mut impl NbtReadHelper<'a>, depth: usize) -> Result<NbtCompound, Error> {
    if depth > MAX_NBT_DEPTH {
        return Err(Error::MaxDepthExceeded);
    }
    let mut compound = NbtCompound::new();
    loop {
        let id = reader.get_u8()?;
        if id == END_ID {
            return Ok(compound);
        }
        let name = reader.get_string()?;
        compound
            .child_tags
            .insert(name.into(), tag(reader, id, depth)?);
    }
}

fn tag<'a>(reader: &mut impl NbtReadHelper<'a>, id: u8, depth: usize) -> Result<NbtTag, Error> {
    // CompoundTag.TYPE and ListTag.TYPE each push exactly one NbtAccounter level.
    match id {
        COMPOUND_ID => compound(reader, depth + 1).map(NbtTag::Compound),
        LIST_ID => list(reader, depth + 1),
        _ => NbtTag::deserialize_data(reader, id),
    }
}

fn list<'a>(reader: &mut impl NbtReadHelper<'a>, depth: usize) -> Result<NbtTag, Error> {
    if depth > MAX_NBT_DEPTH {
        return Err(Error::MaxDepthExceeded);
    }
    let id = reader.get_u8()?;
    let length = reader.get_i32()?;
    if length < 0 {
        return Err(Error::NegativeLength(length));
    }
    if id == END_ID && length > 0 {
        return Err(Error::InvalidListTag(id));
    }
    let length = length as usize;
    if length > MAX_ARRAY_LENGTH {
        return Err(Error::LargeLength(length));
    }
    let mut values = Vec::with_capacity(length.min(4096));
    for _ in 0..length {
        let mut value = tag(reader, id, depth)?;
        // ListTag.tryUnwrap removes one heterogeneous-list wrapper, after reading it.
        if let NbtTag::Compound(compound) = &mut value
            && compound.child_tags.len() == 1
            && let Some(wrapped) = compound.child_tags.remove("")
        {
            value = wrapped;
        }
        values.push(value);
    }
    Ok(NbtTag::List(values))
}
