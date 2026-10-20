use pumpkin_data::{data_component_impl::BannerPatternsImpl, item_stack::ItemStack};
use pumpkin_nbt::{Nbt, NbtCompound, tag::NbtTag};
use pumpkin_util::translation::Locale;

pub(super) fn item_nbt(stack: &ItemStack) -> Nbt {
    let mut tag = NbtCompound::new();
    if let Some(patterns) = stack.get_data_component::<BannerPatternsImpl>() {
        // Geyser BannerItem.convertBannerPattern uses short IDs and reversed dyes.
        let layers = patterns
            .layers
            .iter()
            .filter_map(|layer| {
                let pattern = super::super::banner_pattern::bedrock_pattern(&layer.pattern)?;
                let mut tag = NbtCompound::new();
                tag.put_string("Pattern", pattern.into());
                tag.put_int("Color", 15 - i32::from(layer.color.id()));
                Some(NbtTag::Compound(tag))
            })
            .collect();
        tag.put_list("Patterns", layers);
    }
    if let Some(name) = stack.get_custom_name() {
        let mut display = NbtCompound::new();
        let mut name = name.0.to_bedrock_legacy(Locale::EnUs);
        // The item dialect has a signed int16 UTF-8 byte length. Keep server data intact.
        name.truncate(name.floor_char_boundary(i16::MAX as usize));
        display.put_string("Name", name);
        tag.put_compound("display", display);
    }
    Nbt::new(String::new(), tag)
}

pub(super) fn nbt_bytes(nbt: &Nbt) -> Result<Vec<u8>, std::io::Error> {
    if nbt.is_empty() {
        return Ok(vec![0; 2]);
    }
    let mut bytes = vec![0xff, 0xff, 1];
    // ItemInstanceUserData uses fixed little-endian NBT, unlike block actors.
    bytes.extend_from_slice(
        &nbt.clone()
            .write_fixed_le()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
    );
    Ok(bytes)
}
