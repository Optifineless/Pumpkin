use super::DataComponentCodec;
use crate::{
    VarInt,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};
use pumpkin_data::data_component_impl::{WritableBookContentImpl, WrittenBookContentImpl};
use pumpkin_util::{text::TextComponent, version::JavaMinecraftVersion};

// WritableBookContent and WrittenBookContent constants (vanilla 26.3).
const MAX_PAGES: usize = 100;
const PAGE_EDIT_LENGTH: usize = 1024;
const TITLE_MAX_LENGTH: usize = 32;
const MAX_GENERATION: i32 = 3;
// FriendlyByteBuf.MAX_STRING_LENGTH.
const MAX_STRING_LENGTH: usize = 32767;

fn write_utf8_string(
    seq: &mut impl NetworkWriteExt,
    value: &str,
    max_length: usize,
) -> Result<(), WritingError> {
    // Utf8String.write bounds UTF-16 characters and then UTF-8 bytes, not UTF-8 bytes alone.
    if value.encode_utf16().nth(max_length).is_some() {
        return Err(WritingError::Message(
            "Book string exceeds its length limit".into(),
        ));
    }
    seq.write_string_bounded(value, max_length * 3)
}

impl DataComponentCodec<Self> for WritableBookContentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // WritableBookContent.STREAM_CODEC -> list(100) of Filterable<stringUtf8(1024)>.
        if self.pages.len() > MAX_PAGES {
            return Err(WritingError::Message("Too many writable book pages".into()));
        }
        seq.write_var_int(&VarInt(self.pages.len() as i32))?;
        for page in &self.pages {
            write_utf8_string(seq, page, PAGE_EDIT_LENGTH)?;
            seq.write_bool(false)?;
        }
        Ok(())
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let count = seq.get_var_int()?.0;
        if !(0..=MAX_PAGES as i32).contains(&count) {
            return Err(ReadingError::TooLarge("Writable book pages".into()));
        }
        let mut pages = Vec::with_capacity(crate::ser::collection_capacity(count)?);
        for _ in 0..count {
            let raw = seq.get_str_bounded(PAGE_EDIT_LENGTH)?.to_string();
            if seq.get_bool()? {
                let _ = seq.get_str_bounded(PAGE_EDIT_LENGTH)?;
            }
            pages.push(raw);
        }
        Ok(Self { pages })
    }
}

impl DataComponentCodec<Self> for WrittenBookContentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // WrittenBookContent.STREAM_CODEC: Filterable title, author, generation, pages, resolved.
        write_utf8_string(seq, &self.title, TITLE_MAX_LENGTH)?;
        seq.write_bool(false)?;
        write_utf8_string(seq, &self.author, MAX_STRING_LENGTH)?;
        seq.write_var_int(&VarInt(self.generation))?;
        let count = i32::try_from(self.pages.len())
            .map_err(|_| WritingError::Message("Too many written book pages".into()))?;
        seq.write_var_int(&VarInt(count))?;
        for page in &self.pages {
            seq.write_slice(&page.encode_for_version(&JavaMinecraftVersion::V_26_3))?;
            seq.write_bool(false)?;
        }
        seq.write_bool(self.resolved)
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let title = seq.get_str_bounded(TITLE_MAX_LENGTH)?.to_string();
        if seq.get_bool()? {
            let _ = seq.get_str_bounded(TITLE_MAX_LENGTH)?;
        }
        let author = seq.get_str()?.to_string();
        let generation = seq.get_var_int()?.0;
        if !(0..=MAX_GENERATION).contains(&generation) {
            return Err(ReadingError::Message(
                "Invalid written book generation".into(),
            ));
        }
        let count = seq.get_var_int()?.0;
        if !(0..=crate::MAX_PACKET_DATA_SIZE as i32).contains(&count) {
            return Err(ReadingError::TooLarge("Written book pages".into()));
        }
        // ByteBufCodecs.list initially allocates at most 65536 elements.
        let mut pages = Vec::with_capacity(crate::ser::collection_capacity(count)?);
        for _ in 0..count {
            let tag = seq
                .get_nbt_with_version(&JavaMinecraftVersion::V_26_3)?
                .ok_or_else(|| ReadingError::Message("Missing written book page".into()))?;
            if seq.get_bool()? {
                seq.get_nbt_with_version(&JavaMinecraftVersion::V_26_3)?
                    .ok_or_else(|| ReadingError::Message("Missing filtered book page".into()))?;
            }
            pages.push(TextComponent::from_nbt(&tag));
        }
        let resolved = seq.get_bool()?;
        Ok(Self {
            title,
            author,
            pages,
            generation,
            resolved,
        })
    }
}
