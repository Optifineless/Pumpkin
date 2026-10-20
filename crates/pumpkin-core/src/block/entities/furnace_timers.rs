use pumpkin_nbt::compound::NbtCompound;

/// Reads a vanilla Int timer or an old fork's unsigned Short bit pattern.
#[doc(hidden)]
#[must_use]
pub fn read_timer(nbt: &NbtCompound, name: &str) -> i32 {
    nbt.get_int(name).unwrap_or_else(|| {
        nbt.get_short(name)
            .map_or(0, |value| i32::from(value as u16))
    })
}
