use super::{
    DataComponentCodec, NetworkReadExt, NetworkWriteExt, ReadingError, VarInt, WritingError,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, TooltipDisplayImpl,
        banner_patterns::{
            MAX_BANNER_LAYERS, MAX_PATTERN_STRING_LENGTH, ProjectedBannerPattern,
            banner_pattern_reference, canonical_pattern_identifier,
        },
    },
    dye_color::DyeColor,
};
use std::sync::atomic::{AtomicBool, Ordering};

impl DataComponentCodec<Self> for BannerPatternsImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        static WARNED: AtomicBool = AtomicBool::new(false);
        let layers = self.java_layers().collect::<Vec<_>>();
        if layers.len() != self.layers.len() && !WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "Unsupported saved banner layers omitted from Java projection; stored data retained"
            );
        }
        seq.write_var_int(&VarInt(layers.len() as i32))?;
        for (pattern, color) in layers {
            // ByteBufCodecs.holder: zero has a direct payload; references use ID + 1.
            match pattern {
                ProjectedBannerPattern::Reference { registry_id, .. } => {
                    seq.write_var_int(&VarInt(registry_id + 1))?;
                }
                ProjectedBannerPattern::Direct {
                    asset_id,
                    translation_key,
                } => {
                    seq.write_var_int(&VarInt(0))?;
                    seq.write_string(&asset_id)?;
                    seq.write_string(&translation_key)?;
                }
            }
            seq.write_var_int(&VarInt(i32::from(color.id())))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let count = bounded_count(seq, MAX_BANNER_LAYERS)?;
        let mut layers = Vec::with_capacity(crate::ser::collection_capacity(count)?);
        for _ in 0..count {
            let holder = seq.get_var_int()?.0;
            let (pattern, translation_key) = match holder {
                0 => {
                    let asset = seq.get_str_bounded(MAX_PATTERN_STRING_LENGTH)?;
                    let asset = canonical_pattern_identifier(&asset)
                        .ok_or_else(|| invalid("Invalid banner asset identifier"))?;
                    let key = seq.get_str_bounded(MAX_PATTERN_STRING_LENGTH)?;
                    (asset, Some(key.into()))
                }
                1.. => (
                    banner_pattern_reference(holder - 1)
                        .ok_or_else(|| invalid("Unknown banner holder"))?,
                    None,
                ),
                _ => return Err(invalid("Negative banner holder")),
            };
            let color = u8::try_from(seq.get_var_int()?.0)
                .ok()
                .and_then(DyeColor::by_id)
                .ok_or_else(|| invalid("Invalid banner dye"))?;
            layers.push(BannerPatternLayer {
                pattern,
                color,
                translation_key,
            });
        }
        Ok(Self { layers })
    }
}

fn invalid(message: &str) -> ReadingError {
    ReadingError::Message(message.into())
}

fn bounded_count(seq: &mut impl NetworkReadExt, max: usize) -> Result<usize, ReadingError> {
    let count = seq.get_var_int()?.0;
    let count =
        usize::try_from(count).map_err(|_| invalid("Negative banner/tooltip collection length"))?;
    if count > max {
        return Err(ReadingError::TooLarge("Banner/tooltip collection".into()));
    }
    Ok(count)
}

impl DataComponentCodec<Self> for TooltipDisplayImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_bool(self.hide_tooltip)?;
        seq.write_var_int(&VarInt(self.hidden_components().count() as i32))?;
        for component in self.hidden_components() {
            seq.write_var_int(&VarInt(i32::from(component.to_id())))?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let hide_tooltip = seq.get_bool()?;
        let count = bounded_count(seq, Self::MAX_HIDDEN_COMPONENTS)?;
        let mut hidden_components = Vec::with_capacity(crate::ser::collection_capacity(count)?);
        for _ in 0..count {
            let component = u8::try_from(seq.get_var_int()?.0)
                .ok()
                .and_then(DataComponent::try_from_id)
                .ok_or_else(|| invalid("Invalid hidden component"))?;
            if !hidden_components.contains(&component) {
                hidden_components.push(component);
            }
        }
        Ok(Self {
            hide_tooltip,
            hidden_components,
        })
    }
}
