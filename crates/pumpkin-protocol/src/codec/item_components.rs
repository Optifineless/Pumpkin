use super::{DataComponentCodec, data_to_proto_sound, proto_to_data_sound};
use crate::{
    VarInt,
    codec::item_stack_seralizer::ItemStackSerializer,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};
use pumpkin_data::{
    data_component_impl::{
        ChargedProjectilesImpl, InstrumentImpl, IntangibleProjectileImpl, PaintingVariantImpl,
    },
    instrument::Instrument,
    item_stack::ItemStack,
    painting_variant::PaintingVariant,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::{text::TextComponent, version::JavaMinecraftVersion};
use std::borrow::Cow;

// ChargedProjectiles.MAX_SIZE / STREAM_CODEC (vanilla 26.3).
const MAX_SIZE: i32 = 1024;

impl DataComponentCodec<Self> for ChargedProjectilesImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // Port of upstream #3897: preserve the actual ItemStackTemplate and its patch.
        let count = i32::try_from(self.projectiles.len())
            .map_err(|_| WritingError::Message("Too many charged projectiles".into()))?;
        if count > MAX_SIZE {
            return Err(WritingError::Message("Too many charged projectiles".into()));
        }
        seq.write_var_int(&VarInt(count))?;
        for projectile in &self.projectiles {
            // ItemStackTemplate.MAP_CODEC.optionalFieldOf("count", 1).
            let mut template = Cow::Borrowed(projectile);
            if projectile.get("count").is_none() {
                template.to_mut().put_int("count", 1);
            }
            let stack = ItemStack::read_item_stack(&template)
                .filter(|stack| !stack.is_empty())
                .ok_or_else(|| WritingError::Message("Invalid charged projectile".into()))?;
            ItemStackSerializer(Cow::Owned(stack))
                .write_template_with_version(seq, &JavaMinecraftVersion::V_26_3)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let count = seq.get_var_int()?.0;
        if !(0..=MAX_SIZE).contains(&count) {
            return Err(ReadingError::TooLarge("Charged projectiles".into()));
        }
        let mut projectiles = Vec::with_capacity(crate::ser::collection_capacity(count)?);
        for _ in 0..count {
            let stack = ItemStackSerializer::read_template_with_version(
                seq,
                &JavaMinecraftVersion::V_26_3,
            )?
            .to_stack();
            let mut compound = NbtCompound::new();
            stack.write_item_stack(&mut compound);
            projectiles.push(compound);
        }
        Ok(Self { projectiles })
    }
}

impl DataComponentCodec<Self> for IntangibleProjectileImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // DataComponents.INTANGIBLE_PROJECTILE uses Unit's codec, an empty compound.
        seq.write_nbt(NbtTag::Compound(NbtCompound::new()))
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        seq.get_compound_nbt_with_version(&JavaMinecraftVersion::V_26_3)?
            .ok_or_else(|| {
                ReadingError::Message("Missing intangible projectile compound".into())
            })?;
        Ok(Self)
    }
}

impl DataComponentCodec<Self> for InstrumentImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // Upstream #3348's holder approach, updated to Instrument.STREAM_CODEC in 26.3.
        match self {
            Self::Reference(instrument) => seq.write_var_int(&VarInt(instrument.id() as i32 + 1)),
            Self::Direct {
                sound_event,
                use_duration,
                range,
                durability_damage,
                description,
            } => {
                seq.write_var_int(&VarInt(0))?;
                data_to_proto_sound(sound_event).write(seq, |writer, sound| {
                    writer.write_string(&sound.sound_name)?;
                    writer.write_option(&sound.range, |writer, range| writer.write_f32(*range))
                })?;
                seq.write_f32(*use_duration)?;
                seq.write_f32(*range)?;
                seq.write_var_int(&VarInt(*durability_damage))?;
                seq.write_slice(&description.encode_for_version(&JavaMinecraftVersion::V_26_3))
            }
        }
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let holder = seq.get_var_int()?.0;
        if holder == 0 {
            let sound_event = proto_to_data_sound(&read_instrument_sound(seq)?)
                .ok_or_else(|| ReadingError::Message("Invalid instrument sound".into()))?;
            let use_duration = seq.get_f32()?;
            let range = seq.get_f32()?;
            let durability_damage = seq.get_var_int()?.0;
            let description = seq
                .get_nbt_with_version(&JavaMinecraftVersion::V_26_3)?
                .ok_or_else(|| ReadingError::Message("Missing instrument description".into()))?;
            Ok(Self::Direct {
                sound_event,
                use_duration,
                range,
                durability_damage,
                description: TextComponent::from_nbt(&description),
            })
        } else {
            let index = usize::try_from(holder)
                .ok()
                .and_then(|id| id.checked_sub(1))
                .ok_or_else(|| ReadingError::Message("Invalid instrument holder".into()))?;
            let instrument = Instrument::all()
                .get(index)
                .ok_or_else(|| ReadingError::Message("Unknown instrument holder".into()))?;
            Ok(Self::Reference(*instrument))
        }
    }
}

fn read_instrument_sound(
    seq: &mut impl NetworkReadExt,
) -> Result<crate::IdOr<crate::SoundEvent>, ReadingError> {
    let holder = seq.get_var_int()?.0;
    if holder == 0 {
        Ok(crate::IdOr::Value(crate::SoundEvent {
            sound_name: seq.get_str()?.into(),
            range: seq.get_option(NetworkReadExt::get_f32)?,
        }))
    } else {
        let id = holder
            .checked_sub(1)
            .and_then(|id| u16::try_from(id).ok())
            .ok_or_else(|| ReadingError::Message("Invalid instrument sound holder".into()))?;
        Ok(crate::IdOr::Id(id))
    }
}

impl DataComponentCodec<Self> for PaintingVariantImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // PaintingVariant.STREAM_CODEC -> ByteBufCodecs.holder, not Identifier.STREAM_CODEC.
        let variant = PaintingVariant::from_name(&self.value)
            .ok_or_else(|| WritingError::Message("Unknown painting variant".into()))?;
        seq.write_var_int(&VarInt(variant.id() as i32 + 1))
    }
    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let holder = seq.get_var_int()?.0;
        let index = usize::try_from(holder)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .ok_or_else(|| ReadingError::Message("Invalid painting holder".into()))?;
        let variant = PaintingVariant::all()
            .get(index)
            .ok_or_else(|| ReadingError::Message("Unknown painting holder".into()))?;
        Ok(Self {
            value: Cow::Borrowed(variant.asset_id()),
        })
    }
}
