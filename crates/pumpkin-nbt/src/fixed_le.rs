//! Bedrock's fixed-width little-endian NBT, distinct from network NBT.

use std::{borrow::Cow, io::Write};

use bytes::Bytes;

use crate::{
    COMPOUND_ID, Error, Nbt,
    deserializer::{NbtDataSource, NbtReadHelper},
    serializer::NbtWriteHelper,
};

/// Reads fixed-width little-endian NBT primitives.
pub struct NbtReadHelperFixedLe<D> {
    reader: D,
}

impl<D> NbtReadHelperFixedLe<D> {
    /// Creates a fixed-LE reader over `reader`.
    pub const fn new(reader: D) -> Self {
        Self { reader }
    }
}

macro_rules! read_number {
    ($method:ident, $ty:ty) => {
        fn $method(&mut self) -> Result<$ty, Error> {
            let mut bytes = [0; size_of::<$ty>()];
            self.reader.read_bytes(&mut bytes)?;
            Ok(<$ty>::from_le_bytes(bytes))
        }
    };
}

impl<'a, D: NbtDataSource<'a>> NbtReadHelperFixedLe<D> {
    fn string_len(&mut self) -> Result<u16, Error> {
        let mut bytes = [0; 2];
        self.reader.read_bytes(&mut bytes)?;
        let len = i16::from_le_bytes(bytes);
        u16::try_from(len).map_err(|_| Error::NegativeLength(i32::from(len)))
    }
}

impl<'a, D: NbtDataSource<'a>> NbtReadHelper<'a> for NbtReadHelperFixedLe<D> {
    type Reader = D;

    fn reader(&mut self) -> &mut D {
        &mut self.reader
    }

    fn skip_string(&mut self) -> Result<(), Error> {
        let len = self.string_len()?;
        self.skip_bytes(i64::from(len))
    }

    read_number!(get_u8, u8);
    read_number!(get_i8, i8);
    read_number!(get_i16, i16);
    read_number!(get_i32, i32);
    read_number!(get_i64, i64);
    read_number!(get_f32, f32);
    read_number!(get_f64, f64);

    fn get_string(&mut self) -> Result<Cow<'a, str>, Error> {
        let len = usize::from(self.string_len()?);
        let mut bytes = vec![0; len];
        self.reader.read_bytes(&mut bytes)?;
        String::from_utf8(bytes)
            .map(Cow::Owned)
            .map_err(|_| Error::Utf8DecodingError)
    }

    fn get_byte_array(&mut self, len: usize) -> Result<Cow<'a, [i8]>, Error> {
        self.reader.read_byte_array(len)
    }
}

/// Writes fixed-width little-endian NBT primitives.
pub struct NbtWriteHelperFixedLe<W> {
    writer: W,
}

impl<W: Write> NbtWriteHelperFixedLe<W> {
    /// Creates a fixed-LE writer over `writer`.
    pub const fn new(writer: W) -> Self {
        Self { writer }
    }
}

macro_rules! write_number {
    ($method:ident, $ty:ty) => {
        fn $method(&mut self, value: $ty) -> Result<(), Error> {
            self.writer
                .write_all(&value.to_le_bytes())
                .map_err(Error::Incomplete)
        }
    };
}

impl<W: Write> NbtWriteHelper for NbtWriteHelperFixedLe<W> {
    type Writer = W;

    fn writer(&mut self) -> &mut W {
        &mut self.writer
    }

    write_number!(write_u8, u8);
    write_number!(write_i8, i8);
    write_number!(write_i16, i16);
    write_number!(write_i32, i32);
    write_number!(write_i64, i64);
    write_number!(write_f32, f32);
    write_number!(write_f64, f64);

    fn write_string(&mut self, value: &str) -> Result<(), Error> {
        let len = i16::try_from(value.len()).map_err(|_| Error::LargeLength(value.len()))?;
        self.write_slice(&len.to_le_bytes())?;
        self.write_slice(value.as_bytes())
    }
}

impl Nbt {
    /// Serializes a named document as fixed little-endian NBT for Bedrock items.
    pub fn write_fixed_le(self) -> Result<Bytes, Error> {
        let mut bytes = Vec::new();
        let mut writer = NbtWriteHelperFixedLe::new(&mut bytes);
        writer.write_u8(COMPOUND_ID)?;
        writer.write_string(&self.name)?;
        self.root_tag.serialize_content(&mut writer)?;
        Ok(bytes.into())
    }
}

#[cfg(test)]
#[path = "fixed_le_tests.rs"]
mod tests;
