use super::{
    DataComponentCodec, MAX_STATUS_EFFECTS, data_to_proto_sound, deserialize_idset,
    proto_to_data_sound, serialize_idset,
};
use crate::codec::var_int::VarInt;
use crate::ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError};
use pumpkin_data::data_component_impl::{
    BlockingDamageReduction, BlockingItemDamage, BlocksAttacksImpl, DeathEffect,
    DeathProtectionImpl, DeathStatusEffect, HiddenDeathEffect, IDSetContent, IdOr,
    MAX_DEATH_STATUS_EFFECT_DEPTH, SoundEvent, StatusEffectInstance, WeaponImpl,
};
use pumpkin_data::effect::StatusEffect;
use std::borrow::Cow;

impl DataComponentCodec<Self> for WeaponImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt::from(self.item_damage_per_attack as i32))?;
        seq.write_f32(self.disable_blocking_for_seconds)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let item_damage_per_attack = seq.get_var_int()?.0 as u32;
        let disable_blocking_for_seconds = seq.get_f32()?;
        Ok(Self {
            item_damage_per_attack,
            disable_blocking_for_seconds,
        })
    }
}

impl DataComponentCodec<Self> for DeathProtectionImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        seq.write_var_int(&VarInt(self.death_effects.len() as i32))?;
        for effect in self.death_effects.iter() {
            serialize_death_effect(effect, seq)?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let len = seq.get_var_int()?.0 as usize;
        if len > MAX_STATUS_EFFECTS {
            return Err(ReadingError::Message("Too many death effects".into()));
        }
        let mut death_effects = Vec::with_capacity(len);
        for _ in 0..len {
            death_effects.push(deserialize_death_effect(seq)?);
        }
        Ok(Self {
            death_effects: Cow::Owned(death_effects),
        })
    }
}

// DeathProtection.STREAM_CODEC dispatches ConsumeEffect.STREAM_CODEC by registry id.
fn serialize_death_effect(
    effect: &DeathEffect,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    match effect {
        DeathEffect::ApplyEffects(effects, probability) => {
            seq.write_var_int(&VarInt(0))?;
            seq.write_var_int(&VarInt(effects.len() as i32))?;
            for effect in effects.iter() {
                let id = StatusEffect::from_minecraft_name(&effect.effect.effect_id)
                    .ok_or_else(|| WritingError::Message("Invalid death status effect".into()))?;
                seq.write_var_int(&VarInt(i32::from(id.id)))?;
                serialize_death_effect_details(effect, seq)?;
            }
            seq.write_f32(*probability)
        }
        DeathEffect::RemoveEffects(types) => {
            seq.write_var_int(&VarInt(1))?;
            serialize_idset(types, seq)
        }
        DeathEffect::ClearAllEffects => seq.write_var_int(&VarInt(2)),
        DeathEffect::TeleportRandomly {
            diameter,
            directional_particles,
        } => {
            seq.write_var_int(&VarInt(3))?;
            seq.write_f32(*diameter)?;
            seq.write_bool(*directional_particles)
        }
        DeathEffect::PlaySound(sound) => {
            seq.write_var_int(&VarInt(4))?;
            crate::IdOr::<crate::SoundEvent>::write(
                &data_to_proto_sound(sound),
                seq,
                |seq, event| {
                    seq.write_string(&event.sound_name)?;
                    seq.write_option(&event.range, |seq, range| seq.write_f32(*range))
                },
            )
        }
    }
}

fn serialize_death_effect_details(
    effect: &DeathStatusEffect,
    seq: &mut impl NetworkWriteExt,
) -> Result<(), WritingError> {
    seq.write_var_int(&VarInt(effect.effect.amplifier))?;
    seq.write_var_int(&VarInt(effect.effect.duration))?;
    seq.write_bool(effect.effect.ambient)?;
    seq.write_bool(effect.effect.show_particles)?;
    seq.write_bool(effect.effect.show_icon)?;
    seq.write_bool(effect.hidden_effect.is_some())?;
    if let Some(hidden) = &effect.hidden_effect {
        serialize_death_effect_details(hidden, seq)?;
    }
    Ok(())
}

fn deserialize_death_effect(seq: &mut impl NetworkReadExt) -> Result<DeathEffect, ReadingError> {
    match seq.get_var_int()?.0 {
        0 => {
            let len = seq.get_var_int()?.0 as usize;
            if len > MAX_STATUS_EFFECTS {
                return Err(ReadingError::Message(
                    "Too many death status effects".into(),
                ));
            }
            let mut effects = Vec::with_capacity(len);
            for _ in 0..len {
                let id = u16::try_from(seq.get_var_int()?.0)
                    .map_err(|_| ReadingError::Message("Invalid death status effect id".into()))?;
                let effect = StatusEffect::from_id(id).ok_or_else(|| {
                    ReadingError::Message("Invalid death status effect id".into())
                })?;
                effects.push(deserialize_death_effect_details(
                    seq,
                    effect.minecraft_name,
                    0,
                )?);
            }
            Ok(DeathEffect::ApplyEffects(
                Cow::Owned(effects),
                seq.get_f32()?,
            ))
        }
        1 => Ok(DeathEffect::RemoveEffects(deserialize_idset(seq)?)),
        2 => Ok(DeathEffect::ClearAllEffects),
        3 => Ok(DeathEffect::TeleportRandomly {
            diameter: seq.get_f32()?,
            directional_particles: seq.get_bool()?,
        }),
        4 => Ok(DeathEffect::PlaySound(deserialize_blocking_sound(seq)?)),
        _ => Err(ReadingError::Message(
            "Invalid death consume effect id".into(),
        )),
    }
}

fn deserialize_death_effect_details(
    seq: &mut impl NetworkReadExt,
    id: &'static str,
    depth: usize,
) -> Result<DeathStatusEffect, ReadingError> {
    if depth > MAX_DEATH_STATUS_EFFECT_DEPTH {
        return Err(ReadingError::Message(
            "Death status effect nesting is too deep".into(),
        ));
    }
    let effect = StatusEffectInstance {
        effect_id: Cow::Borrowed(id),
        amplifier: seq.get_var_int()?.0.clamp(0, i32::from(u8::MAX)),
        duration: seq.get_var_int()?.0,
        ambient: seq.get_bool()?,
        show_particles: seq.get_bool()?,
        show_icon: seq.get_bool()?,
    };
    let hidden_effect = if seq.get_bool()? {
        Some(HiddenDeathEffect::Owned(Box::new(
            deserialize_death_effect_details(seq, id, depth + 1)?,
        )))
    } else {
        None
    };
    Ok(DeathStatusEffect {
        effect,
        hidden_effect,
    })
}

impl DataComponentCodec<Self> for BlocksAttacksImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // BlocksAttacks.STREAM_CODEC, including three FLOATs for ItemDamageFunction.
        seq.write_f32(self.block_delay_seconds)?;
        seq.write_f32(self.disable_cooldown_scale)?;
        seq.write_var_int(&VarInt(self.damage_reductions.len() as i32))?;
        for reduction in self.damage_reductions.iter() {
            seq.write_f32(reduction.horizontal_blocking_angle)?;
            seq.write_option(&reduction.damage_type, |seq, types| {
                serialize_idset(types, seq)
            })?;
            seq.write_f32(reduction.base)?;
            seq.write_f32(reduction.factor)?;
        }
        seq.write_f32(self.item_damage.threshold)?;
        seq.write_f32(self.item_damage.base)?;
        seq.write_f32(self.item_damage.factor)?;
        seq.write_option(&self.bypassed_by, |seq, types| serialize_idset(types, seq))?;
        for sound in [&self.block_sound, &self.disable_sound] {
            seq.write_option(sound, |seq, sound| {
                crate::IdOr::<crate::SoundEvent>::write(
                    &data_to_proto_sound(sound),
                    seq,
                    |seq, event| {
                        seq.write_string(&event.sound_name)?;
                        seq.write_option(&event.range, |seq, range| seq.write_f32(*range))
                    },
                )
            })?;
        }
        Ok(())
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let block_delay_seconds = seq.get_f32()?;
        let disable_cooldown_scale = seq.get_f32()?;
        let red_len = seq.get_var_int()?.0 as usize;
        if red_len > MAX_STATUS_EFFECTS {
            return Err(ReadingError::Message(
                "Too many blocking damage reductions".into(),
            ));
        }
        let mut damage_reductions = Vec::with_capacity(red_len);
        for _ in 0..red_len {
            damage_reductions.push(BlockingDamageReduction {
                horizontal_blocking_angle: seq.get_f32()?,
                damage_type: seq.get_option(deserialize_idset)?,
                base: seq.get_f32()?,
                factor: seq.get_f32()?,
            });
        }
        let item_damage = BlockingItemDamage {
            threshold: seq.get_f32()?,
            base: seq.get_f32()?,
            factor: seq.get_f32()?,
        };
        let bypassed_by = seq.get_option(deserialize_idset)?;
        let block_sound = seq.get_option(deserialize_blocking_sound)?;
        let disable_sound = seq.get_option(deserialize_blocking_sound)?;
        Ok(Self {
            block_delay_seconds,
            disable_cooldown_scale,
            damage_reductions: Cow::Owned(damage_reductions),
            item_damage,
            bypassed_by,
            block_sound,
            disable_sound,
        })
    }
}

fn deserialize_blocking_sound(
    seq: &mut impl NetworkReadExt,
) -> Result<IdOr<SoundEvent>, ReadingError> {
    // ByteBufCodecs.holderRegistry: validate before narrowing the registry id.
    let holder = seq.get_var_int()?.0;
    if holder == 0 {
        return Ok(IdOr::Value(SoundEvent {
            sound_name: Cow::Owned(seq.get_str()?.into()),
            range: seq.get_option(NetworkReadExt::get_f32)?,
        }));
    }
    let id = u16::try_from(
        holder
            .checked_sub(1)
            .ok_or_else(|| ReadingError::Message("Invalid blocking sound holder".into()))?,
    )
    .map_err(|_| ReadingError::Message("Invalid blocking sound holder".into()))?;
    proto_to_data_sound(&crate::IdOr::Id(id))
        .ok_or_else(|| ReadingError::Message("Invalid blocking sound".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::damage::DamageType;
    use pumpkin_data::data_component_impl::IDSet;
    use pumpkin_data::sound::Sound;

    // Vanilla 26.3 Weapon.STREAM_CODEC: VarInt durability, float disable seconds.
    #[test]
    fn weapon_preserves_vanilla_disable_duration() {
        let bytes = [0xac, 0x02, 0x40, 0xa0, 0, 0];
        let decoded = WeaponImpl::deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded.item_damage_per_attack, 300);
        assert_eq!(decoded.disable_blocking_for_seconds, 5.0);
        let mut encoded = Vec::new();
        decoded.serialize(&mut encoded).unwrap();
        assert_eq!(encoded, bytes);
    }

    #[test]
    fn blocking_preserves_vanilla_float_function_and_optional_holders() {
        // BlocksAttacks.STREAM_CODEC and DamageReduction.STREAM_CODEC field order.
        let mut bytes = vec![
            0x3e, 0x80, 0, 0, // delay 0.25
            0x3f, 0, 0, 0, // disable scale 0.5
            1, // one reduction
            0x42, 0x34, 0, 0, // angle 45
            1, 2, 34, // type present: one holder, player_attack
            0x3f, 0x80, 0, 0, // base 1
            0x3e, 0x00, 0, 0, // factor 0.125
            0x40, 0x40, 0, 0, // durability threshold 3
            0x40, 0x00, 0, 0, // durability base 2
            0x3f, 0x40, 0, 0, // durability factor 0.75
            1, 0, 25, // bypassed_by present: tag, UTF-8 length
        ];
        bytes.extend_from_slice(b"minecraft:bypasses_shield");
        bytes.extend_from_slice(&[1, 0, 12]); // block sound present: inline, name length
        bytes.extend_from_slice(b"example:test");
        bytes.extend_from_slice(&[1, 0x41, 0x80, 0, 0]); // range present: 16
        bytes.extend_from_slice(&[1, 1]); // disable sound present: registered holder 0
        let mut input = bytes.as_slice();
        let decoded = BlocksAttacksImpl::deserialize(&mut input).unwrap();
        assert!(input.is_empty());
        assert_eq!(decoded.block_delay_seconds, 0.25);
        assert_eq!(decoded.disable_cooldown_scale, 0.5);
        assert_eq!(decoded.damage_reductions.len(), 1);
        let reduction = &decoded.damage_reductions[0];
        assert_eq!(reduction.horizontal_blocking_angle, 45.0);
        assert_eq!((reduction.base, reduction.factor), (1.0, 0.125));
        assert!(matches!(
            &reduction.damage_type,
            Some(IDSet::IDs(types))
                if types.len() == 1 && types[0].damage_type == DamageType::PLAYER_ATTACK
        ));
        assert_eq!(
            decoded.item_damage,
            BlockingItemDamage {
                threshold: 3.0,
                base: 2.0,
                factor: 0.75
            }
        );
        assert!(matches!(
            &decoded.bypassed_by,
            Some(IDSet::Tag(tag)) if tag.as_ref() == "minecraft:bypasses_shield"
        ));
        assert_eq!(
            decoded.block_sound,
            Some(IdOr::Value(SoundEvent::new(
                "example:test".into(),
                Some(16.0)
            )))
        );
        assert_eq!(
            decoded.disable_sound,
            Some(IdOr::Id(Sound::EntityAllayAmbientWithItem))
        );
        let mut encoded = Vec::new();
        decoded.serialize(&mut encoded).unwrap();
        assert_eq!(encoded, bytes);
    }

    #[test]
    fn death_protection_preserves_ordered_vanilla_consume_effects() {
        // DeathProtection/ApplyStatusEffectsConsumeEffect/MobEffectInstance.STREAM_CODEC.
        let bytes = [
            5, 2, // five effects, first clear_all_effects
            0, 1, // apply_effects, one status effect
            9, 1, 0x84, 7, // regeneration, amplifier 1, duration 900
            0, 0, 1, 1, // ambient, particles, icon, hidden effect present
            0, 100, 1, 0, 0,
            0, // hidden details: amplifier, duration, three flags, no nested effect
            0x3f, 0x80, 0, 0, // probability 1 (after the effect list)
            3, 0x41, 0x80, 0, 0, 0, // teleport diameter 16, directional_particles false
            1, 3, 0, 9, // remove_effects: holder set of speed and regeneration
            4, 1, // play_sound: registered holder 0
        ];
        let mut input = bytes.as_slice();
        let decoded = DeathProtectionImpl::deserialize(&mut input).unwrap();
        assert!(input.is_empty());
        assert_eq!(decoded.death_effects.len(), 5);
        assert!(matches!(
            decoded.death_effects[0],
            DeathEffect::ClearAllEffects
        ));
        assert!(matches!(
            &decoded.death_effects[1],
            DeathEffect::ApplyEffects(effects, probability)
                if effects.len() == 1 && *probability == 1.0
        ));
        if let DeathEffect::ApplyEffects(effects, _) = &decoded.death_effects[1] {
            let visible = &effects[0].effect;
            assert_eq!(visible.effect_id, "minecraft:regeneration");
            assert_eq!((visible.amplifier, visible.duration), (1, 900));
            assert_eq!(
                (visible.ambient, visible.show_particles, visible.show_icon),
                (false, false, true)
            );
            let hidden = effects[0].hidden_effect.as_ref().unwrap();
            assert_eq!(hidden.effect.effect_id, "minecraft:regeneration");
            assert_eq!((hidden.effect.amplifier, hidden.effect.duration), (0, 100));
            assert_eq!(
                (
                    hidden.effect.ambient,
                    hidden.effect.show_particles,
                    hidden.effect.show_icon
                ),
                (true, false, false)
            );
            assert!(hidden.hidden_effect.is_none());
        }
        assert!(matches!(
            &decoded.death_effects[2],
            DeathEffect::TeleportRandomly { diameter, directional_particles }
                if *diameter == 16.0 && !*directional_particles
        ));
        assert!(matches!(
            &decoded.death_effects[3],
            DeathEffect::RemoveEffects(IDSet::IDs(types))
                if types.len() == 2
                    && types[0] == &StatusEffect::SPEED
                    && types[1] == &StatusEffect::REGENERATION
        ));
        assert_eq!(
            decoded.death_effects[4],
            DeathEffect::PlaySound(IdOr::Id(Sound::EntityAllayAmbientWithItem))
        );
        let mut encoded = Vec::new();
        decoded.serialize(&mut encoded).unwrap();
        assert_eq!(encoded, bytes);
    }
    #[test]
    fn network_amplifiers_clamp_visible_and_hidden_effects() {
        // Details.STREAM_CODEC permits VarInts; MobEffectInstance clamps each constructor.
        let bytes = [
            0xac, 0x02, 1, 0, 1, 1, 1, 0xff, 0xff, 0xff, 0xff, 0x0f, 2, 0, 1, 1, 0,
        ];
        let effect =
            deserialize_death_effect_details(&mut bytes.as_slice(), "minecraft:regeneration", 0)
                .unwrap();
        assert_eq!(effect.effect.amplifier, 255);
        assert_eq!(effect.hidden_effect.unwrap().effect.amplifier, 0);
    }

    #[test]
    fn sound_holders_do_not_alias_when_the_id_exceeds_u16() {
        for bytes in [
            vec![0x81, 0x80, 0x04],
            vec![0xff, 0xff, 0xff, 0xff, 0x0f],
            vec![0x80, 0x80, 0x80, 0x80, 0x08],
        ] {
            assert!(deserialize_blocking_sound(&mut bytes.as_slice()).is_err());
        }
    }

    #[test]
    fn use_effects_preserves_suppressed_vibrations() {
        use pumpkin_data::data_component_impl::UseEffectsImpl;
        let bytes = [1, 0, 0x3f, 0x40, 0, 0];
        let effects = UseEffectsImpl::deserialize(&mut bytes.as_slice()).unwrap();
        assert!(effects.can_sprint);
        assert!(!effects.interact_vibrations);
        assert_eq!(effects.speed_multiplier, 0.75);
        let mut encoded = Vec::new();
        effects.serialize(&mut encoded).unwrap();
        assert_eq!(encoded, bytes);
    }
}
