use super::{component_codec, display_projection};
use pumpkin_nbt::{NbtCompound, serializer::NbtWriteHelperJava, tag::NbtTag};

#[test]
fn display_projection_bounds_encoded_bytes_including_modified_utf8_and_empty_nodes() {
    let mut empty_forest = NbtCompound::new();
    empty_forest.put_string("text", String::new());
    empty_forest.put_list("extra", vec![NbtTag::Compound(NbtCompound::new()); 20_000]);
    let mut translation = NbtCompound::new();
    translation.put_string("translate", "custom.key".into());
    translation.put_list("with", vec![NbtTag::String("A".repeat(60_000).into()); 140]);
    for input in [
        NbtTag::String("A".repeat(60_000).into()),
        NbtTag::String("\0😀".repeat(7_000).into()),
        NbtTag::Compound(empty_forest),
        NbtTag::Compound(translation),
    ] {
        let projected = display_projection::project(&input, "");
        let tag = component_codec::write_component(&projected);
        let mut encoded = Vec::new();
        tag.serialize_data(&mut NbtWriteHelperJava::new(&mut encoded))
            .unwrap();
        assert!(
            encoded.len() <= 32 * 1024,
            "Encoded display has {} bytes",
            encoded.len()
        );
    }
}
