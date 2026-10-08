use super::*;

fn saved_book(title: &str, pages: Vec<NbtTag>) -> NbtTag {
    let mut nbt = NbtCompound::new();
    nbt.put_string("title", title.into());
    nbt.put_string("author", "A".into());
    nbt.put_list("pages", pages);
    NbtTag::Compound(nbt)
}

#[test]
fn loaded_book_bounds_use_utf16_units() {
    // Independent NBT inputs, with exact and overflowing supplementary-character boundaries.
    for (title, accepted) in [("😀".repeat(16), true), ("😀".repeat(17), false)] {
        assert_eq!(
            WrittenBookContentImpl::read_data(&saved_book(&title, vec![])).is_some(),
            accepted
        );
    }
    for (text, accepted) in [("😀".repeat(512), true), ("😀".repeat(513), false)] {
        let nbt = saved_book("T", vec![NbtTag::String(text.into())]);
        assert_eq!(WritableBookContentImpl::read_data(&nbt).is_some(), accepted);
        assert_eq!(WrittenBookContentImpl::read_data(&nbt).is_some(), accepted);
    }
    for (count, accepted) in [(100, true), (101, false)] {
        let nbt = saved_book("T", vec![NbtTag::String("page".into()); count]);
        assert_eq!(WritableBookContentImpl::read_data(&nbt).is_some(), accepted);
        assert_eq!(WrittenBookContentImpl::read_data(&nbt).is_some(), accepted);
    }
}
