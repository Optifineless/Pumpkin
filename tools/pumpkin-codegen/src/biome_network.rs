use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

use crate::environment_attribute::network_attribute_type;

/// Applies Biome.NETWORK_CODEC to the extracted biome's NBT before registry generation.
pub(crate) fn encode(biome: &mut NbtCompound) {
    biome.child_tags.retain(|key, _| {
        matches!(
            key.as_ref(),
            "has_precipitation"
                | "temperature"
                | "temperature_modifier"
                | "downfall"
                | "effects"
                | "attributes"
        )
    });
    // Biome.ClimateSettings.CODEC uses Codec.FLOAT and omits TemperatureModifier.NONE.
    for key in ["temperature", "downfall"] {
        if let Some(value) = biome.child_tags.get_mut(key) {
            encode_float(value);
        }
    }
    omit_default(biome, "temperature_modifier", "none");
    if let Some(NbtTag::Compound(effects)) = biome.child_tags.get_mut("effects") {
        // BiomeSpecialEffects.CODEC omits GrassColorModifier.NONE.
        omit_default(effects, "grass_color_modifier", "none");
    }
    if let Some(NbtTag::Compound(attributes)) = biome.child_tags.get_mut("attributes") {
        // EnvironmentAttributeMap.NETWORK_CODEC filters EnvironmentAttribute.isSyncable.
        attributes.child_tags.retain(|name, value| {
            let Some(attribute_type) = network_attribute_type(name) else {
                return false;
            };
            // EnvironmentAttributeMap.Entry.createCodec supports a direct value or modifier.
            let argument = if let NbtTag::Compound(entry) = value
                && entry.get("modifier").is_some()
            {
                entry.child_tags.get_mut("argument")
            } else {
                Some(value)
            };
            if let Some(argument) = argument {
                match attribute_type {
                    "Float" | "AngleDegrees" => encode_float(argument),
                    "AmbientParticles" => encode_particles(argument),
                    _ => {}
                }
            }
            true
        });
    }
}

fn omit_default(compound: &mut NbtCompound, key: &str, default: &str) {
    if matches!(compound.get(key), Some(NbtTag::String(value)) if &**value == default) {
        compound.child_tags.remove(key);
    }
}

fn encode_float(value: &mut NbtTag) {
    *value = NbtTag::Float(match value {
        NbtTag::Int(value) => *value as f32,
        NbtTag::Long(value) => *value as f32,
        NbtTag::Double(value) => *value as f32,
        NbtTag::Float(value) => *value,
        _ => panic!("Expected a Codec.FLOAT value in extracted biome data"),
    });
}

fn encode_particles(value: &mut NbtTag) {
    if let NbtTag::List(particles) = value {
        for particle in particles {
            if let NbtTag::Compound(particle) = particle
                && let Some(probability) = particle.child_tags.get_mut("probability")
            {
                // AmbientParticle.CODEC uses Codec.floatRange for probability.
                encode_float(probability);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::encode;
    use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

    #[test]
    fn explicit_default_biome_modifiers_are_omitted() {
        let mut biome = NbtCompound::new();
        biome.put_string("temperature_modifier", "none".into());
        let mut effects = NbtCompound::new();
        effects.put_string("grass_color_modifier", "none".into());
        biome.put_compound("effects", effects);

        encode(&mut biome);

        assert!(biome.get("temperature_modifier").is_none());
        let Some(NbtTag::Compound(effects)) = biome.get("effects") else {
            panic!("missing effects");
        };
        assert!(effects.get("grass_color_modifier").is_none());
    }
}
