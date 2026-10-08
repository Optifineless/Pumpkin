use pumpkin_data::{Block, item::Item, item_stack::ItemStack, particle::Particle};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::codec::particle_options::{ParticleOptions, VibrationDestination};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    version::JavaMinecraftVersion,
};

// ScalableParticleOptionsBase.MIN_SCALE / MAX_SCALE (Java lines 8-9).
const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 4.0;

// ParticleArgument.readParticle dispatches the corresponding ParticleType.codec over SNBT.
pub(super) fn encode(particle: Particle, nbt: &NbtCompound) -> Option<Vec<u8>> {
    let options = match particle {
        Particle::Block
        | Particle::BlockMarker
        | Particle::FallingDust
        | Particle::DustPillar
        | Particle::BlockCrumble => ParticleOptions::Block(block_state(nbt.get("block_state")?)?),
        Particle::Item => {
            let item = item_template(nbt.get("item")?)?;
            return ParticleOptions::Item(&item)
                .encode(&JavaMinecraftVersion::V_26_3)
                .ok();
        }
        Particle::Dust => ParticleOptions::Dust {
            color: color(nbt.get("color")?, false)?,
            scale: scale(nbt)?,
        },
        Particle::DustColorTransition => ParticleOptions::DustColorTransition {
            from_color: color(nbt.get("from_color")?, false)?,
            to_color: color(nbt.get("to_color")?, false)?,
            scale: scale(nbt)?,
        },
        Particle::EntityEffect | Particle::Flash | Particle::TintedLeaves => {
            ParticleOptions::Color(color(nbt.get("color")?, true)?)
        }
        Particle::Effect | Particle::InstantEffect => ParticleOptions::Spell {
            color: match nbt.get("color") {
                Some(tag) => color(tag, false)?,
                None => -1,
            },
            power: optional_float(nbt, "power", 1.0)?,
        },
        Particle::DragonBreath => ParticleOptions::Power(optional_float(nbt, "power", 1.0)?),
        Particle::SculkCharge => ParticleOptions::SculkCharge(nbt.get("roll")?.as_numeric_float()?),
        Particle::Trail => ParticleOptions::Trail {
            target: vector(nbt.get("target")?)?,
            color: color(nbt.get("color")?, false)?,
            duration: positive(nbt, "duration")?,
        },
        Particle::Shriek => ParticleOptions::Shriek(integer(nbt.get("delay")?)?),
        Particle::Vibration => ParticleOptions::Vibration {
            destination: destination(nbt.get_compound("destination")?)?,
            arrival_in_ticks: integer(nbt.get("arrival_in_ticks")?)?,
        },
        Particle::Geyser | Particle::GeyserPlume => {
            ParticleOptions::Geyser(positive(nbt, "water_blocks")?)
        }
        Particle::GeyserBase | Particle::GeyserPoof => ParticleOptions::GeyserBase {
            water_blocks: positive(nbt, "water_blocks")?,
            burst_impulse_base: nbt.get("burst_impulse_base")?.as_numeric_float()?,
        },
        _ => return Some(Vec::new()), // SimpleParticleType.codec is MapCodec.unit.
    };
    options.encode(&JavaMinecraftVersion::V_26_3).ok()
}

fn integer(tag: &NbtTag) -> Option<i32> {
    // Codec.INT/NbtOps uses Number.intValue, including narrowing a LongTag.
    match tag {
        NbtTag::Long(number) => Some(*number as i32),
        _ => tag.as_numeric_double().map(|number| number as i32),
    }
}

fn positive(nbt: &NbtCompound, key: &str) -> Option<i32> {
    integer(nbt.get(key)?).filter(|value| *value > 0)
}

fn optional_float(nbt: &NbtCompound, key: &str, default: f32) -> Option<f32> {
    nbt.get(key).map_or(Some(default), NbtTag::as_numeric_float)
}

fn scale(nbt: &NbtCompound) -> Option<f32> {
    // ScalableParticleOptionsBase.SCALE rejects command values outside MIN_SCALE..MAX_SCALE.
    nbt.get("scale")?
        .as_numeric_float()
        .filter(|value| (MIN_SCALE..=MAX_SCALE).contains(value))
}

fn vector(tag: &NbtTag) -> Option<Vector3<f64>> {
    let [x, y, z] = tag.extract_list()? else {
        return None;
    };
    Some(Vector3::new(
        x.as_numeric_double()?,
        y.as_numeric_double()?,
        z.as_numeric_double()?,
    ))
}

fn color(tag: &NbtTag, alpha: bool) -> Option<i32> {
    // ExtraCodecs.RGB_COLOR_CODEC / ARGB_COLOR_CODEC accept an integer or a float vector.
    if let Some(number) = integer(tag) {
        return Some(number);
    }
    let values = tag.extract_list()?;
    if values.len() != if alpha { 4 } else { 3 } {
        return None;
    }
    let mut channels = values
        .iter()
        // ARGB.as8BitChannel converts a float channel to eight bits with floor.
        .map(|tag| {
            tag.as_numeric_float()
                .map(|v| (v * f32::from(u8::MAX)).floor() as i32)
        });
    let red = channels.next()??;
    let green = channels.next()??;
    let blue = channels.next()??;
    let alpha = if alpha {
        channels.next()??
    } else {
        i32::from(u8::MAX)
    };
    Some((alpha << 24) | (red << 16) | (green << 8) | blue)
}

fn block_state(tag: &NbtTag) -> Option<u32> {
    // BlockParticleOption.BLOCK_STATE_CODEC accepts a block name or BlockState.CODEC's compound.
    let (name, properties) = match tag {
        NbtTag::String(name) => (name.as_ref(), None),
        NbtTag::Compound(nbt) => (nbt.get_string("Name")?, nbt.get_compound("Properties")),
        _ => return None,
    };
    let block = Block::from_name(name)?;
    let state = if let Some(properties) = properties {
        let mut values = block.properties(block.default_state.id)?.to_props();
        for (key, value) in &properties.child_tags {
            let property = values.iter_mut().find(|(name, _)| *name == key.as_ref())?;
            property.1 = value.extract_string()?;
        }
        block.state_from_properties(&values)?
    } else {
        block.default_state
    };
    Some(u32::from(state.id.as_u16()))
}

fn item_template(tag: &NbtTag) -> Option<ItemStack> {
    // ItemStackTemplate.CODEC accepts bare IDs or MAP_CODEC with count defaulting to one.
    let stack = match tag {
        NbtTag::String(name) => ItemStack::new(1, Item::from_registry_key(name)?),
        NbtTag::Compound(nbt) => {
            let mut nbt = nbt.clone();
            let count = match nbt.get("count") {
                Some(tag) => integer(tag)?,
                None => 1,
            };
            if !(1..=99).contains(&count) {
                return None;
            }
            nbt.put_int("count", count);
            ItemStack::read_item_stack(&nbt)?
        }
        _ => return None,
    };
    (!stack.is_empty()).then_some(stack)
}

fn destination(nbt: &NbtCompound) -> Option<VibrationDestination> {
    // VibrationParticleOption.SAFE_POSITION_SOURCE_CODEC disallows entity destinations.
    if !matches!(nbt.get_string("type")?, "minecraft:block" | "block") {
        return None;
    }
    let pos = nbt.get("pos")?;
    let coords: [i32; 3] = if let Some(array) = pos.extract_int_array() {
        array.try_into().ok()?
    } else {
        let [x, y, z] = pos.extract_list()? else {
            return None;
        };
        [integer(x)?, integer(y)?, integer(z)?]
    };
    Some(VibrationDestination::Block(BlockPos::new(
        coords[0], coords[1], coords[2],
    )))
}
