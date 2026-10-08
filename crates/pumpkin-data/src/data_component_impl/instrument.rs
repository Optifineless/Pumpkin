use super::{DataComponentImpl, IdOr, SoundEvent};
use crate::{instrument::Instrument, sound::Sound};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::text::TextComponent;
use std::borrow::Cow;

#[path = "instrument_hash.rs"]
mod hash;

#[derive(Clone, Debug, PartialEq)]
pub enum InstrumentImpl {
    Reference(Instrument),
    Direct {
        sound_event: IdOr<SoundEvent>,
        use_duration: f32,
        range: f32,
        durability_damage: i32,
        description: TextComponent,
    },
}

impl InstrumentImpl {
    // InstrumentComponent.CODEC -> Instrument.CODEC accepts a reference or an inline definition.
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        match data {
            NbtTag::String(name) => Instrument::from_name(name).map(Self::Reference),
            NbtTag::Compound(compound) => {
                let sound_event = match compound.get("sound_event")? {
                    NbtTag::String(name) => IdOr::Id(Sound::from_name(
                        name.strip_prefix("minecraft:").unwrap_or(name),
                    )?),
                    NbtTag::Compound(sound) => IdOr::Value(SoundEvent {
                        sound_name: Cow::Owned(sound.get_string("sound_id")?.into()),
                        range: sound.get("range").and_then(number),
                    }),
                    _ => return None,
                };
                let use_duration = number(compound.get("use_duration")?)?;
                let range = number(compound.get("range")?)?;
                let durability_damage = match compound.get("durability_damage") {
                    Some(tag) => integer(tag)?,
                    None => 0,
                };
                if !use_duration.is_finite()
                    || use_duration < 0.0
                    || !range.is_finite()
                    || range <= 0.0
                    || durability_damage < 0
                {
                    return None;
                }
                Some(Self::Direct {
                    sound_event,
                    use_duration,
                    range,
                    durability_damage,
                    description: TextComponent::from_nbt(compound.get("description")?),
                })
            }
            _ => None,
        }
    }
}

// NbtOps.getNumberValue lets Instrument.DIRECT_CODEC accept any numeric NBT type.
fn number(tag: &NbtTag) -> Option<f32> {
    Some(match tag {
        NbtTag::Byte(value) => f32::from(*value),
        NbtTag::Short(value) => f32::from(*value),
        NbtTag::Int(value) => *value as f32,
        NbtTag::Long(value) => *value as f32,
        NbtTag::Float(value) => *value,
        NbtTag::Double(value) => *value as f32,
        _ => return None,
    })
}

fn integer(tag: &NbtTag) -> Option<i32> {
    Some(match tag {
        NbtTag::Byte(value) => i32::from(*value),
        NbtTag::Short(value) => i32::from(*value),
        NbtTag::Int(value) => *value,
        NbtTag::Long(value) => *value as i32,
        NbtTag::Float(value) => *value as i32,
        NbtTag::Double(value) => *value as i32,
        _ => return None,
    })
}

impl DataComponentImpl for InstrumentImpl {
    fn get_hash(&self) -> i32 {
        // InstrumentComponent.CODEC -> RegistryFileCodec.encode -> HashOps.CRC32C_INSTANCE.
        match self {
            Self::Reference(instrument) => super::get_str_hash(instrument.asset_id()) as i32,
            Self::Direct {
                durability_damage,
                description,
                ..
            } => {
                let NbtTag::Compound(mut compound) = self.write_data() else {
                    return 0;
                };
                if *durability_damage == 0 {
                    compound.child_tags.remove("durability_damage");
                }
                compound.put(
                    "description",
                    description.to_nbt_tag_for_version(
                        &pumpkin_util::version::JavaMinecraftVersion::V_26_3,
                    ),
                );
                hash::hash_tag(&NbtTag::Compound(compound)) as i32
            }
        }
    }
    fn write_data(&self) -> NbtTag {
        match self {
            Self::Reference(instrument) => NbtTag::String(instrument.asset_id().into()),
            Self::Direct {
                sound_event,
                use_duration,
                range,
                durability_damage,
                description,
            } => {
                let mut compound = NbtCompound::new();
                super::put_idor(&mut compound, "sound_event", sound_event);
                compound.put_float("use_duration", *use_duration);
                compound.put_float("range", *range);
                compound.put_int("durability_damage", *durability_damage);
                compound.put(
                    "description",
                    NbtTag::Compound(description.0.to_nbt_compound()),
                );
                NbtTag::Compound(compound)
            }
        }
    }
    default_impl!(Instrument);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrument_hash_matches_vanilla_hashops_fixtures() {
        // Fixtures obtained with vanilla 26.3 HashOps; RegistryFileCodec encodes references as names.
        assert_eq!(
            InstrumentImpl::Reference(Instrument::PonderGoatHorn).get_hash() as u32,
            0xbd0b99fa
        );
        assert_eq!(
            InstrumentImpl::Reference(Instrument::SingGoatHorn).get_hash() as u32,
            0xb298dccc
        );
        // Instrument.DIRECT_CODEC omits default durability_damage=0 and encodes plain text as STRING.
        let inline = InstrumentImpl::Direct {
            sound_event: IdOr::Id(Sound::ItemGoatHornSound0),
            use_duration: 1.0,
            range: 2.0,
            durability_damage: 0,
            description: TextComponent::text("D"),
        };
        assert_eq!(inline.get_hash() as u32, 0x77bb9081);
    }
}
