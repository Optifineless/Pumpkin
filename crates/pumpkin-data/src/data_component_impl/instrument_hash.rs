use super::super::Digest;
use pumpkin_nbt::tag::NbtTag;

// HashOps.createMap/createList/createNumeric/createString for Instrument.DIRECT_CODEC.
pub(in crate::data_component_impl) fn hash_tag(tag: &NbtTag) -> u32 {
    let mut digest = Digest::new();
    match tag {
        NbtTag::End => digest.update(&[1]),
        NbtTag::Byte(value) => digest.update(&[6, *value as u8]),
        NbtTag::Short(value) => {
            digest.update(&[7]);
            digest.update(&value.to_le_bytes());
        }
        NbtTag::Int(value) => {
            digest.update(&[8]);
            digest.update(&value.to_le_bytes());
        }
        NbtTag::Long(value) => {
            digest.update(&[9]);
            digest.update(&value.to_le_bytes());
        }
        NbtTag::Float(value) => {
            digest.update(&[10]);
            digest.update(&value.to_le_bytes());
        }
        NbtTag::Double(value) => {
            digest.update(&[11]);
            digest.update(&value.to_le_bytes());
        }
        NbtTag::String(value) => {
            digest.update(&[12]);
            digest.update(&(value.encode_utf16().count() as u32).to_le_bytes());
            for unit in value.encode_utf16() {
                digest.update(&unit.to_le_bytes());
            }
        }
        NbtTag::List(values) => {
            digest.update(&[4]);
            for value in values {
                digest.update(&hash_tag(value).to_le_bytes());
            }
            digest.update(&[5]);
        }
        NbtTag::Compound(compound) => return hash_compound(compound),
        NbtTag::ByteArray(values) => {
            digest.update(&[14]);
            for value in values {
                digest.update(&[*value as u8]);
            }
            digest.update(&[15]);
        }
        NbtTag::IntArray(values) => {
            digest.update(&[16]);
            for value in values {
                digest.update(&value.to_le_bytes());
            }
            digest.update(&[17]);
        }
        NbtTag::LongArray(values) => {
            digest.update(&[18]);
            for value in values {
                digest.update(&value.to_le_bytes());
            }
            digest.update(&[19]);
        }
    }
    digest.finalize() as u32
}

fn hash_compound(compound: &pumpkin_nbt::compound::NbtCompound) -> u32 {
    let mut digest = Digest::new();
    let mut fields: Vec<_> = compound
        .child_tags
        .iter()
        .map(|(key, value)| {
            let value_hash = if matches!(
                key.as_ref(),
                "bold"
                    | "italic"
                    | "underlined"
                    | "strikethrough"
                    | "obfuscated"
                    | "interpret"
                    | "hat"
            ) {
                // Style.CODEC uses BOOL; NBT stores those booleans as bytes.
                value.extract_bool().map_or_else(
                    || hash_tag(value),
                    |value| crc32c::crc32c(&[13, u8::from(value)]),
                )
            } else {
                hash_tag(value)
            };
            (hash_tag(&NbtTag::String(key.clone())), value_hash)
        })
        .collect();
    // HashCode.padToLong compares these CRC32C values as unsigned integers.
    fields.sort_unstable();
    digest.update(&[2]);
    for (key, value) in fields {
        digest.update(&key.to_le_bytes());
        digest.update(&value.to_le_bytes());
    }
    digest.update(&[3]);
    digest.finalize() as u32
}
