use std::io::Cursor;

use super::*;
use crate::{NbtCompound, tag::NbtTag};

#[test]
fn fixed_le_primitives_match_protocol_bytes() {
    let mut bytes = Vec::new();
    let mut writer = NbtWriteHelperFixedLe::new(&mut bytes);
    writer.write_i8(-2).unwrap();
    writer.write_i16(-3).unwrap();
    writer.write_i32(0x12345678).unwrap();
    writer.write_i64(-4).unwrap();
    writer.write_f32(1.5).unwrap();
    writer.write_f64(-2.5).unwrap();
    writer.write_string("é🎨").unwrap();
    assert_eq!(
        bytes,
        [
            0xfe, 0xfd, 0xff, 0x78, 0x56, 0x34, 0x12, 0xfc, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0, 0, 0xc0, 0x3f, 0, 0, 0, 0, 0, 0, 4, 0xc0, 6, 0, 0xc3, 0xa9, 0xf0, 0x9f, 0x8e,
            0xa8,
        ]
    );
    let mut reader = NbtReadHelperFixedLe::new(Cursor::new(bytes.as_slice()));
    assert_eq!(reader.get_i8().unwrap(), -2);
    assert_eq!(reader.get_i16().unwrap(), -3);
    assert_eq!(reader.get_i32().unwrap(), 0x12345678);
    assert_eq!(reader.get_i64().unwrap(), -4);
    assert_eq!(reader.get_f32().unwrap(), 1.5);
    assert_eq!(reader.get_f64().unwrap(), -2.5);
    assert_eq!(reader.get_string().unwrap(), "é🎨");
    assert!(reader.get_u8().is_err());
}

#[test]
fn fixed_le_document_round_trips_lists_arrays_and_compounds() {
    let mut root = NbtCompound::new();
    root.put_byte("byte", -1);
    root.put_short("short", i16::MIN);
    root.put_int("int", i32::MIN);
    root.put_long("long", i64::MIN);
    root.put_float("float", 1.5);
    root.put_double("double", -2.5);
    root.put_string("text", "é🎨".into());
    root.put("bytes", NbtTag::ByteArray(vec![-1, 0, 127].into()));
    root.put("ints", NbtTag::IntArray(vec![i32::MIN, -1, 0, i32::MAX]));
    root.put("longs", NbtTag::LongArray(vec![i64::MIN, -1, 0, i64::MAX]));
    root.put_list("empty", vec![]);
    root.put_list("list", vec![NbtTag::Int(-1), NbtTag::Int(1234)]);
    root.put_compound("compound", NbtCompound::new());
    let document = Nbt::new("root".into(), root);
    let bytes = document.clone().write_fixed_le().unwrap();
    assert_eq!(&bytes[..7], &[10, 4, 0, b'r', b'o', b'o', b't']);
    let mut cursor = Cursor::new(bytes.as_ref());
    assert_eq!(
        Nbt::read(&mut NbtReadHelperFixedLe::new(&mut cursor)).unwrap(),
        document
    );
    assert_eq!(cursor.position() as usize, bytes.len());
}

#[test]
fn fixed_le_rejects_invalid_or_oversized_strings() {
    let mut writer = NbtWriteHelperFixedLe::new(Vec::new());
    assert!(matches!(
        writer.write_string(&"x".repeat(65536)),
        Err(Error::LargeLength(65536))
    ));
    assert!(matches!(
        writer.write_string(&"x".repeat(32768)),
        Err(Error::LargeLength(32768))
    ));
    for bytes in [&[2, 0, b'a'][..], &[1, 0, 0xff][..], &[0, 0x80][..]] {
        assert!(
            NbtReadHelperFixedLe::new(Cursor::new(bytes))
                .get_string()
                .is_err()
        );
    }
}

#[test]
fn fixed_le_document_errors_do_not_return_partial_bytes() {
    let mut root = NbtCompound::new();
    root.put_string("Name", "x".repeat(32768));
    assert!(matches!(
        Nbt::new(String::new(), root).write_fixed_le(),
        Err(Error::LargeLength(32768))
    ));
}
