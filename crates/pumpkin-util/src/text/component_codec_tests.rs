use super::TextComponent;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};

#[test]
fn review3_shared_decoder_preserves_translation_arguments_and_fallback() {
    for arguments in [None, Some(NbtTag::List(vec![NbtTag::Int(1)]))] {
        let mut nbt = NbtCompound::new();
        nbt.put_string("translate", "unknown.key".into());
        nbt.put_string("fallback", "Key %s".into());
        if let Some(arguments) = arguments {
            nbt.put("with", arguments);
        }
        let tag = NbtTag::Compound(nbt);
        let decoded = TextComponent::from_nbt(&tag);
        assert_eq!(
            decoded.0.to_nbt_compound(),
            tag.extract_compound().unwrap().clone()
        );
    }
}
