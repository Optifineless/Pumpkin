use super::{NetworkReadExt, ReadingError};
use pumpkin_nbt::{
    deserializer::{NbtReadHelper, NbtReadHelperJava},
    tag::NbtTag,
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::collections::HashSet;

// NbtAccounter.DEFAULT_NBT_QUOTA and MAX_STACK_DEPTH. Validate before allocating tags.
pub(super) const DEFAULT_NBT_QUOTA: usize = 2_097_152;
pub(super) const MAX_STACK_DEPTH: usize = 512;

struct Scanner<'a, R: NetworkReadExt + ?Sized> {
    read: &'a mut R,
    bytes: Vec<u8>,
    usage: usize,
}

impl<R: NetworkReadExt + ?Sized> Scanner<'_, R> {
    fn account(&mut self, count: usize) -> Result<(), ReadingError> {
        self.usage = self
            .usage
            .checked_add(count)
            .filter(|usage| *usage <= DEFAULT_NBT_QUOTA)
            .ok_or_else(|| ReadingError::TooLarge("NBT quota".into()))?;
        Ok(())
    }

    fn byte(&mut self) -> Result<u8, ReadingError> {
        let byte = self.read.get_u8()?;
        self.bytes.push(byte);
        Ok(byte)
    }

    fn payload(&mut self, mut size: usize) -> Result<(), ReadingError> {
        let mut buffer = [0; 4096];
        while size > 0 {
            let length = size.min(buffer.len());
            self.read.read_bytes_to_buf(&mut buffer[..length])?;
            self.bytes.extend_from_slice(&buffer[..length]);
            size -= length;
        }
        Ok(())
    }

    fn length(&mut self) -> Result<usize, ReadingError> {
        let value = self.read.get_i32_be()?;
        self.bytes.extend_from_slice(&value.to_be_bytes());
        usize::try_from(value).map_err(|_| ReadingError::Message("Negative NBT length".into()))
    }

    fn string(&mut self, overhead: usize) -> Result<String, ReadingError> {
        let length = self.read.get_u16_be()?;
        self.bytes.extend_from_slice(&length.to_be_bytes());
        let start = self.bytes.len();
        self.payload(usize::from(length))?;
        let value = cesu8::from_java_cesu8(&self.bytes[start..])
            .map_err(|_| ReadingError::Message("Invalid NBT string".into()))?
            .into_owned();
        // StringTag.load / CompoundTag.readString account UTF-16, not encoded bytes.
        self.account(overhead + 2 * value.encode_utf16().count())?;
        Ok(value)
    }

    fn root_name(&mut self) -> Result<(), ReadingError> {
        // NbtIo.readUnnamedTag / StringTag.skipString do not charge the discarded root name.
        let length = self.read.get_u16_be()?;
        self.bytes.extend_from_slice(&length.to_be_bytes());
        self.payload(usize::from(length))
    }

    fn array(&mut self, width: usize) -> Result<(), ReadingError> {
        let length = self.length()?;
        let bytes = length
            .checked_mul(width)
            .ok_or_else(|| ReadingError::TooLarge("NBT array".into()))?;
        // ByteArrayTag/IntArrayTag/LongArrayTag.load account before allocating.
        self.account(24)?;
        self.account(bytes)?;
        self.payload(bytes)
    }

    fn tag(&mut self, id: u8, depth: usize) -> Result<(), ReadingError> {
        // The TagType.load implementations account their Java heap footprint.
        match id {
            0 => self.account(8),
            1 => {
                self.account(9)?;
                self.payload(1)
            }
            2 => {
                self.account(10)?;
                self.payload(2)
            }
            3 | 5 => {
                self.account(12)?;
                self.payload(4)
            }
            4 | 6 => {
                self.account(16)?;
                self.payload(8)
            }
            7 => self.array(1),
            8 => self.string(36).map(|_| ()),
            9 | 10 if depth >= MAX_STACK_DEPTH => Err(ReadingError::TooLarge("NBT depth".into())),
            9 => self.list(depth + 1),
            10 => self.compound(depth + 1),
            11 => self.array(4),
            12 => self.array(8),
            _ => Err(ReadingError::Message("Unknown NBT tag".into())),
        }
    }

    fn list(&mut self, depth: usize) -> Result<(), ReadingError> {
        self.account(36)?;
        let id = self.byte()?;
        let count = self.length()?;
        if id == 0 && count > 0 {
            return Err(ReadingError::Message("Nonempty end-tag list".into()));
        }
        self.account(
            count
                .checked_mul(4)
                .ok_or_else(|| ReadingError::TooLarge("NBT list".into()))?,
        )?;
        for _ in 0..count {
            self.tag(id, depth)?;
        }
        Ok(())
    }

    fn compound(&mut self, depth: usize) -> Result<(), ReadingError> {
        self.account(48)?;
        let mut keys = HashSet::new();
        loop {
            let id = self.byte()?;
            if id == 0 {
                return Ok(());
            }
            let key = self.string(28)?;
            self.tag(id, depth)?;
            if keys.insert(key) {
                self.account(36)?;
            }
        }
    }
}

pub(super) fn read_network_nbt(
    read: &mut (impl NetworkReadExt + ?Sized),
    version: JavaMinecraftVersion,
) -> Result<Option<NbtTag>, ReadingError> {
    let mut scanner = Scanner {
        read,
        bytes: Vec::new(),
        usage: 0,
    };
    let id = scanner.byte()?;
    if id == 0 {
        return Ok(None);
    }
    if version < JavaMinecraftVersion::V_1_20_2 {
        scanner.root_name()?;
    }
    scanner.tag(id, 0)?;
    let mut cursor = std::io::Cursor::new(scanner.bytes.as_slice());
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    helper
        .get_u8()
        .map_err(|err| ReadingError::Message(err.to_string()))?;
    if version < JavaMinecraftVersion::V_1_20_2 {
        helper
            .skip_string()
            .map_err(|err| ReadingError::Message(err.to_string()))?;
    }
    super::nbt_reader::BudgetedNbtReader::new()
        .tag(&mut helper, id, 0)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_nbt_depth_and_byte_array_boundaries_decode() {
        let mut bytes = vec![10];
        for _ in 0..511 {
            bytes.extend_from_slice(&[10, 0, 0]);
        }
        bytes.extend_from_slice(&[0; 512]);
        assert!(read_network_nbt(&mut bytes.as_slice(), JavaMinecraftVersion::V_26_3).is_ok());

        // ByteArrayTag.load: 24 bytes overhead, so 2 MiB - 24 bytes is exactly legal.
        let length = 2_097_128i32;
        let mut bytes = vec![7];
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.resize(5 + length as usize, 0);
        assert!(read_network_nbt(&mut bytes.as_slice(), JavaMinecraftVersion::V_26_3).is_ok());
        bytes[1..5].copy_from_slice(&(length + 1).to_be_bytes());
        assert!(matches!(
            read_network_nbt(&mut bytes.as_slice(), JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
    }

    #[test]
    fn named_root_does_not_reduce_the_legal_tag_quota() {
        // Named compound "A", empty-key byte array. Java charges 136 bytes plus its data.
        let length = 2_097_016i32;
        let mut bytes = vec![10, 0, 1, b'A', 7, 0, 0];
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.resize(11 + length as usize, 0);
        bytes.push(0);
        assert!(read_network_nbt(&mut bytes.as_slice(), JavaMinecraftVersion::V_1_8).is_ok());
        let mut cursor = std::io::Cursor::new(bytes.as_slice());
        let mut helper = NbtReadHelperJava::new(&mut cursor);
        assert!(super::super::nbt_reader::read_named_nbt(&mut helper).is_ok());
    }

    #[test]
    fn nbt_allocation_quota_is_checked_before_array_data() {
        // TAG_Long_Array, count 262,144. Java heap charge exceeds 2 MiB before payload.
        let mut input: &[u8] = &[12, 0, 4, 0, 0];
        assert!(matches!(
            read_network_nbt(&mut input, JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
        // TAG_Byte_Array with i32::MAX entries must never allocate its declared size.
        let mut input: &[u8] = &[7, 0x7f, 0xff, 0xff, 0xff];
        assert!(matches!(
            read_network_nbt(&mut input, JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
    }

    #[test]
    fn nbt_depth_is_bounded_before_deserializing() {
        let mut bytes = vec![10];
        for _ in 0..512 {
            bytes.extend_from_slice(&[10, 0, 0]);
        }
        bytes.extend_from_slice(&[0; 513]);
        assert!(matches!(
            read_network_nbt(&mut bytes.as_slice(), JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
    }

    #[test]
    fn pre_18_gzip_nbt_uses_heap_quota_for_borrowed_and_stream_readers() {
        use crate::ser::NetworkReadSliceExt;
        use std::io::Write;
        // Named root compound, list "a", byte elements, 524289 entries; no element bytes.
        let raw = [10, 0, 0, 9, 0, 1, b'a', 1, 0, 8, 0, 1];
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&raw).unwrap();
        let compressed = gzip.finish().unwrap();
        let mut packet = (compressed.len() as i16).to_be_bytes().to_vec();
        packet.extend_from_slice(&compressed);
        assert!(matches!(
            packet
                .as_slice()
                .get_nbt_borrowed(&JavaMinecraftVersion::V_1_7_6),
            Err(ReadingError::TooLarge(_))
        ));
        let mut stream = std::io::Cursor::new(packet);
        assert!(matches!(
            stream.get_nbt_with_version(&JavaMinecraftVersion::V_1_7_6),
            Err(ReadingError::TooLarge(_))
        ));
    }
}
