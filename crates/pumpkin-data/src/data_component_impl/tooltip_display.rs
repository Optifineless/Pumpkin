use super::{DataComponentImpl, Digest, get_str_hash, instrument::hash::hash_tag};
use crate::data_component::DataComponent;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[derive(Clone, Default)]
pub struct TooltipDisplayImpl {
    pub hide_tooltip: bool,
    pub hidden_components: Vec<DataComponent>,
}

impl PartialEq for TooltipDisplayImpl {
    fn eq(&self, other: &Self) -> bool {
        self.hide_tooltip == other.hide_tooltip
            && self.hidden_components().count() == other.hidden_components().count()
            && self
                .hidden_components()
                .all(|component| other.hidden_components.contains(&component))
    }
}

impl Eq for TooltipDisplayImpl {}

impl std::hash::Hash for TooltipDisplayImpl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        self.hide_tooltip.hash(state);
        let mut components: Vec<_> = self.hidden_components().map(|c| c.to_id()).collect();
        components.sort_unstable();
        components.hash(state);
    }
}

// Keep existing generated item defaults and native value construction compatible.
#[expect(
    non_upper_case_globals,
    reason = "Preserve the former unit-struct value"
)]
pub const TooltipDisplayImpl: TooltipDisplayImpl = TooltipDisplayImpl {
    hide_tooltip: false,
    hidden_components: Vec::new(),
};

impl std::fmt::Debug for TooltipDisplayImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TooltipDisplayImpl")
            .field("hide_tooltip", &self.hide_tooltip)
            .field(
                "hidden_components",
                &self
                    .hidden_components
                    .iter()
                    .map(|c| c.to_name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl TooltipDisplayImpl {
    pub const MAX_HIDDEN_COMPONENTS: usize = u8::MAX as usize + 1;

    pub fn hidden_components(&self) -> impl Iterator<Item = DataComponent> + '_ {
        self.hidden_components
            .iter()
            .enumerate()
            .filter_map(|(i, component)| {
                (!self.hidden_components[..i].contains(component)).then_some(*component)
            })
    }

    pub fn read_data(data: &NbtTag) -> Option<Self> {
        // The old unit implementation wrote End; older empty compounds mean DEFAULT too.
        if matches!(data, NbtTag::End) {
            return Some(TooltipDisplayImpl);
        }
        let tag = data.extract_compound()?;
        let hide_tooltip = match tag.get("hide_tooltip") {
            Some(value) => value.extract_bool()?,
            None => false,
        };
        let mut hidden_components = Vec::new();
        if let Some(value) = tag.get("hidden_components") {
            let NbtTag::List(list) = value else {
                return None;
            };
            if list.len() > Self::MAX_HIDDEN_COMPONENTS {
                return None;
            }
            for entry in list {
                let component = DataComponent::try_from_name(entry.extract_string()?)?;
                if !hidden_components.contains(&component) {
                    hidden_components.push(component);
                }
            }
        }
        Some(Self {
            hide_tooltip,
            hidden_components,
        })
    }
}

impl DataComponentImpl for TooltipDisplayImpl {
    fn write_data(&self) -> NbtTag {
        let mut tag = NbtCompound::new();
        if self.hide_tooltip {
            tag.put_bool("hide_tooltip", true);
        }
        let hidden = self
            .hidden_components()
            .map(|c| NbtTag::String(c.to_name().into()))
            .collect::<Vec<_>>();
        if !hidden.is_empty() {
            tag.put_list("hidden_components", hidden);
        }
        NbtTag::Compound(tag)
    }

    fn get_hash(&self) -> i32 {
        let mut fields = Vec::new();
        if self.hide_tooltip {
            // TooltipDisplay.CODEC uses BOOL, whose HashOps tag is 13, rather than BYTE.
            fields.push((get_str_hash("hide_tooltip"), crc32c::crc32c(&[13, 1])));
        }
        let hidden = self
            .hidden_components()
            .map(|c| NbtTag::String(c.to_name().into()))
            .collect::<Vec<_>>();
        if !hidden.is_empty() {
            fields.push((
                get_str_hash("hidden_components"),
                hash_tag(&NbtTag::List(hidden)),
            ));
        }
        fields.sort_unstable();
        let mut digest = Digest::new();
        digest.update(&[2]);
        for (key, value) in fields {
            digest.update(&key.to_le_bytes());
            digest.update(&value.to_le_bytes());
        }
        digest.update(&[3]);
        digest.finalize() as i32
    }

    default_impl!(TooltipDisplay);
}
