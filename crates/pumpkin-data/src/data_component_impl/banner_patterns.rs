use super::{DataComponentImpl, instrument::hash::hash_tag};
use crate::{dye_color::DyeColor, registry::REGISTRY_V_26_3};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::identifier::Identifier;

/// Resource bound for untrusted Java input, independent of saved layers and publication.
pub const MAX_BANNER_LAYERS: usize = 1024;
pub const MAX_PATTERN_STRING_LENGTH: usize = 32767;

/// The reference name, or a direct holder's asset ID and translation key.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct BannerPatternLayer {
    pub pattern: String,
    pub color: DyeColor,
    pub translation_key: Option<String>,
}

/// A Java-visible holder; unsupported saved references have no projection.
pub enum ProjectedBannerPattern {
    Reference {
        registry_id: i32,
        name: String,
    },
    Direct {
        asset_id: String,
        translation_key: String,
    },
}

pub fn canonical_pattern_identifier(value: &str) -> Option<String> {
    if value.len() > MAX_PATTERN_STRING_LENGTH {
        return None;
    }
    let id = Identifier::parse(value).ok()?;
    // Identifier.isValidNamespace in 26.3 also excludes exactly "..".
    let canonical = id.to_string();
    (id.namespace() != ".." && canonical.len() <= MAX_PATTERN_STRING_LENGTH).then_some(canonical)
}

pub fn banner_pattern_reference(registry_id: i32) -> Option<String> {
    let entries = REGISTRY_V_26_3
        .iter()
        .find(|r| r.registry_id == "banner_pattern")?
        .entries;
    let entry = entries.get(usize::try_from(registry_id).ok()?)?;
    canonical_pattern_identifier(entry.name)
}

impl BannerPatternLayer {
    /// Mirrors BannerPattern.STREAM_CODEC using the synchronized vanilla prefix.
    #[must_use]
    pub fn java_projection(&self) -> Option<ProjectedBannerPattern> {
        let name = canonical_pattern_identifier(&self.pattern)?;
        if let Some(translation_key) = &self.translation_key {
            if translation_key.encode_utf16().count() > MAX_PATTERN_STRING_LENGTH {
                return None;
            }
            return Some(ProjectedBannerPattern::Direct {
                asset_id: name,
                translation_key: translation_key.clone(),
            });
        }
        let entries = REGISTRY_V_26_3
            .iter()
            .find(|r| r.registry_id == "banner_pattern")?
            .entries;
        let registry_id = entries.iter().position(|entry| {
            canonical_pattern_identifier(entry.name).as_deref() == Some(name.as_str())
        })? as i32;
        Some(ProjectedBannerPattern::Reference { registry_id, name })
    }

    fn read_data(tag: &NbtTag) -> Option<Self> {
        let layer = tag.extract_compound()?;
        // Preserve the old saved reader's fallback for unrecognized color names.
        let color = DyeColor::by_name(layer.get_string("color")?).unwrap_or_default();
        let (pattern, translation_key) = match layer.get("pattern")? {
            NbtTag::String(name) => {
                // Compatibility input from the old PR's flattened direct form.
                if let Some(key) = layer.get_string("translation_key")
                    && let Some(pattern) = canonical_pattern_identifier(name)
                {
                    (pattern, Some(key.to_owned()))
                } else {
                    (name.to_string(), None)
                }
            }
            NbtTag::Compound(direct) => (
                canonical_pattern_identifier(direct.get_string("asset_id")?)?,
                Some(direct.get_string("translation_key")?.to_owned()),
            ),
            _ => return None,
        };
        Some(Self {
            pattern,
            color,
            translation_key,
        })
    }

    fn write_data(&self) -> NbtTag {
        let mut layer = NbtCompound::new();
        if let Some(key) = &self.translation_key {
            let mut direct = NbtCompound::new();
            direct.put_string("asset_id", self.pattern.clone());
            direct.put_string("translation_key", key.clone());
            layer.put("pattern", NbtTag::Compound(direct));
        } else {
            layer.put_string("pattern", self.pattern.clone());
        }
        layer.put_string("color", self.color.name().to_string());
        NbtTag::Compound(layer)
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct BannerPatternsImpl {
    pub layers: Vec<BannerPatternLayer>,
}

impl BannerPatternsImpl {
    pub const EMPTY: Self = Self { layers: Vec::new() };

    #[must_use]
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let NbtTag::List(list) = data else {
            return Some(Self::EMPTY);
        };
        // BannerPatternLayers.CODEC uses an unrestricted list; retain legacy saved layers.
        Some(Self {
            layers: list
                .iter()
                .filter(|tag| matches!(tag, NbtTag::Compound(_)))
                .map(BannerPatternLayer::read_data)
                .collect::<Option<_>>()?,
        })
    }

    pub fn java_layers(&self) -> impl Iterator<Item = (ProjectedBannerPattern, DyeColor)> + '_ {
        self.layers
            .iter()
            .filter_map(|layer| Some((layer.java_projection()?, layer.color)))
    }
}

impl DataComponentImpl for BannerPatternsImpl {
    fn write_data(&self) -> NbtTag {
        NbtTag::List(
            self.layers
                .iter()
                .map(BannerPatternLayer::write_data)
                .collect(),
        )
    }

    fn get_hash(&self) -> i32 {
        // BannerPatternLayers.Layer.CODEC hashes exactly the layers sent to Java.
        let layers = self
            .java_layers()
            .map(|(pattern, color)| {
                let (pattern, translation_key) = match pattern {
                    ProjectedBannerPattern::Reference { name, .. } => (name, None),
                    ProjectedBannerPattern::Direct {
                        asset_id,
                        translation_key,
                    } => (asset_id, Some(translation_key)),
                };
                BannerPatternLayer {
                    pattern,
                    translation_key,
                    color,
                }
                .write_data()
            })
            .collect();
        hash_tag(&NbtTag::List(layers)) as i32
    }

    default_impl!(BannerPatterns);
}
