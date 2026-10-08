use crate::data_component_impl::DataComponentImpl;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;

// WritableBookContent and WrittenBookContent's title constants (vanilla 26.3).
const MAX_PAGES: usize = 100;
const PAGE_EDIT_LENGTH: usize = 1024;
const TITLE_MAX_LENGTH: usize = 32;
// FriendlyByteBuf.MAX_STRING_LENGTH, used by WrittenBookContent.STREAM_CODEC's author.
const MAX_STRING_LENGTH: usize = 32767;

/// Reads a filterable text, which vanilla stores either as a plain string or as
/// a compound with a `raw` field, e.g. `{raw: "A Partner"}`. The raw value is a
/// text component of its own, so it may also be a styled compound such as
/// `{raw: {text: "A Partner", color: "red"}}`.
fn read_component(tag: &NbtTag) -> Option<TextComponent> {
    match tag {
        NbtTag::String(value) => Some(TextComponent::text(value.to_string())),
        NbtTag::Compound(compound) => {
            let component = compound.get("raw").unwrap_or(tag);
            Some(TextComponent::from_nbt(component))
        }
        _ => None,
    }
}

/// Reads a filterable text whose content is a plain string, like writable book
/// pages and written book titles in vanilla.
fn read_text(tag: &NbtTag) -> Option<String> {
    read_component(tag).map(TextComponent::get_text)
}

fn text_tag(value: &str) -> NbtTag {
    // Filterable.codec(Codec.STRING), used by WritableBookContent.CODEC and written titles.
    let mut compound = NbtCompound::new();
    compound.put_string("raw", value.to_string());
    NbtTag::Compound(compound)
}

fn text_component_tag(component: &TextComponent) -> NbtTag {
    let mut compound = NbtCompound::new();
    compound.put("raw", NbtTag::Compound(component.0.to_nbt_compound()));
    NbtTag::Compound(compound)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WritableBookContentImpl {
    pub pages: Vec<String>,
}
impl WritableBookContentImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut pages = Vec::new();
        if let NbtTag::Compound(c) = tag
            && let Some(NbtTag::List(l)) = c.get("pages")
        {
            if l.len() > MAX_PAGES {
                return None;
            }
            for item in l {
                if let Some(page) = read_text(item) {
                    // WritableBookContent.CODEC uses Java String.length (UTF-16).
                    if page.encode_utf16().nth(PAGE_EDIT_LENGTH).is_some() {
                        return None;
                    }
                    pages.push(page);
                }
            }
        }
        Some(Self { pages })
    }
}
impl DataComponentImpl for WritableBookContentImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        let pages_tags: Vec<NbtTag> = self.pages.iter().map(|p| text_tag(p)).collect();
        compound.put("pages", NbtTag::List(pages_tags));
        NbtTag::Compound(compound)
    }
    default_impl!(WritableBookContent);
}

/// Vanilla `WrittenBookContent` stores pages as filterable components, so the
/// parsed component is kept instead of being flattened to plain text.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WrittenBookContentImpl {
    pub title: String,
    pub author: String,
    pub pages: Vec<TextComponent>,
}
impl WrittenBookContentImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut pages = Vec::new();
        let mut title = String::new();
        let mut author = String::new();
        if let NbtTag::Compound(c) = tag {
            if let Some(value) = c.get("title").and_then(read_text) {
                // WrittenBookContent.CODEC's Filterable<Codec.string(0, 32)>.
                if value.encode_utf16().nth(TITLE_MAX_LENGTH).is_some() {
                    return None;
                }
                title = value;
            }
            if let Some(s) = c.get_string("author") {
                if s.encode_utf16().nth(MAX_STRING_LENGTH).is_some() {
                    return None;
                }
                author = s.to_string();
            }
            if let Some(NbtTag::List(l)) = c.get("pages") {
                // Fork load policy: apply editable-book page limits to signed books too.
                if l.len() > MAX_PAGES {
                    return None;
                }
                for item in l {
                    if let Some(page) = read_component(item) {
                        if page
                            .clone()
                            .get_text()
                            .encode_utf16()
                            .nth(PAGE_EDIT_LENGTH)
                            .is_some()
                        {
                            return None;
                        }
                        pages.push(page);
                    }
                }
            }
        }
        Some(Self {
            title,
            author,
            pages,
        })
    }
}
impl DataComponentImpl for WrittenBookContentImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("title", text_tag(&self.title));
        compound.put_string("author", self.author.clone());
        let pages_tags: Vec<NbtTag> = self.pages.iter().map(text_component_tag).collect();
        compound.put("pages", NbtTag::List(pages_tags));
        NbtTag::Compound(compound)
    }
    default_impl!(WrittenBookContent);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DebugStickStateImpl;
impl DebugStickStateImpl {
    pub const fn read_data(_data: &NbtTag) -> Option<Self> {
        Some(Self)
    }
}
impl DataComponentImpl for DebugStickStateImpl {
    default_impl!(DebugStickState);
}

#[cfg(test)]
#[path = "book_load_tests.rs"]
mod load_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_util::text::color::{Color, NamedColor};

    fn page_texts(pages: &[TextComponent]) -> Vec<String> {
        pages.iter().map(|page| page.clone().get_text()).collect()
    }

    fn compound(values: &[(&str, NbtTag)]) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (key, value) in values {
            compound.put(key, value.clone());
        }
        NbtTag::Compound(compound)
    }

    fn raw_list(values: &[&str]) -> NbtTag {
        NbtTag::List(values.iter().map(|v| text_tag(v)).collect())
    }

    #[test]
    fn written_book_preserves_styled_raw_components() {
        let mut styled = NbtCompound::new();
        styled.put_string("text", "hi".to_string());
        styled.put_string("color", "red".to_string());
        let mut wrapper = NbtCompound::new();
        wrapper.put("raw", NbtTag::Compound(styled));
        let tag = compound(&[
            ("title", text_tag("Styled")),
            ("pages", NbtTag::List(vec![NbtTag::Compound(wrapper)])),
        ]);
        let content = WrittenBookContentImpl::read_data(&tag).unwrap();
        assert_eq!(page_texts(&content.pages), ["hi"]);
        assert_eq!(
            content.pages[0].0.style.color,
            Some(Color::Named(NamedColor::Red))
        );

        let read_back = WrittenBookContentImpl::read_data(&content.write_data()).unwrap();
        assert_eq!(page_texts(&read_back.pages), ["hi"]);
        assert_eq!(
            read_back.pages[0].0.style.color,
            Some(Color::Named(NamedColor::Red))
        );
    }

    #[test]
    fn written_book_reads_raw_pages_and_title() {
        let tag = compound(&[
            ("title", text_tag("A Partner")),
            ("author", NbtTag::String("Dylan Collins".into())),
            ("pages", raw_list(&["first page", "second page"])),
        ]);
        let content = WrittenBookContentImpl::read_data(&tag).unwrap();
        assert_eq!(content.title, "A Partner");
        assert_eq!(content.author, "Dylan Collins");
        assert_eq!(page_texts(&content.pages), ["first page", "second page"]);
    }

    #[test]
    fn written_book_still_reads_plain_strings() {
        let tag = compound(&[
            ("title", NbtTag::String("Plain".into())),
            (
                "pages",
                NbtTag::List(vec![NbtTag::String("plain page".into())]),
            ),
        ]);
        let content = WrittenBookContentImpl::read_data(&tag).unwrap();
        assert_eq!(content.title, "Plain");
        assert_eq!(page_texts(&content.pages), ["plain page"]);
    }

    #[test]
    fn writable_book_reads_raw_pages() {
        let tag = compound(&[("pages", raw_list(&["hello"]))]);
        let content = WritableBookContentImpl::read_data(&tag).unwrap();
        assert_eq!(content.pages, ["hello"]);
    }

    #[test]
    fn plain_filterable_values_are_written_as_strings() {
        // Filterable.codec(Codec.STRING): titles and editable pages are strings, not components.
        let title = text_tag("Title");
        let NbtTag::Compound(wrapper) = title else {
            panic!("expected filterable wrapper");
        };
        assert_eq!(wrapper.get_string("raw"), Some("Title"));
        let writable = WritableBookContentImpl {
            pages: vec!["one".into(), "two".into()],
        };
        let NbtTag::Compound(saved) = writable.write_data() else {
            panic!("expected book compound");
        };
        let pages = saved.get_list("pages").unwrap();
        assert_eq!(pages.len(), 2);
        for (page, expected) in pages.iter().zip(["one", "two"]) {
            let NbtTag::Compound(wrapper) = page else {
                panic!("expected filterable page")
            };
            assert_eq!(wrapper.get_string("raw"), Some(expected));
        }
    }

    #[test]
    fn written_book_round_trip_keeps_pages() {
        let content = WrittenBookContentImpl {
            title: "Title".to_string(),
            author: "Author".to_string(),
            pages: vec![TextComponent::text("one"), TextComponent::text("two")],
        };
        let tag = content.write_data();
        let written_compound = match &tag {
            NbtTag::Compound(c) => c,
            _ => panic!("expected a compound"),
        };
        assert!(matches!(
            written_compound.get("title"),
            Some(NbtTag::Compound(_))
        ));
        let read_back = WrittenBookContentImpl::read_data(&tag).unwrap();
        assert_eq!(read_back, content);
    }
}
