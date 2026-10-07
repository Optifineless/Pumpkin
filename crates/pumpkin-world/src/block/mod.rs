use std::collections::HashMap;

use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::generation::structure::template::{BlockStateResolver, PaletteEntry};

/// Encodes a block state as a chunk palette compound.
#[must_use]
pub fn block_state_to_nbt(id: BlockStateId) -> NbtCompound {
    let block = Block::from_state_id(id);
    let mut compound = NbtCompound::new();
    compound.put_string("Name", block_state_name(block));
    if let Some(properties) = block_state_properties_to_nbt(block, id) {
        compound.put_compound("Properties", properties);
    }
    compound
}

/// Decodes a block state compound, returning air for missing or unknown blocks.
#[must_use]
pub fn block_state_from_nbt(compound: &NbtCompound) -> BlockStateId {
    PaletteEntry::from_nbt_compound(compound)
        .ok()
        .and_then(|entry| BlockStateResolver::resolve_simple(&entry))
        .map_or(BlockStateId::AIR, |state| state.id)
}

/// Encodes a block state using vanilla's `BlockState.CODEC`.
#[must_use]
pub fn block_state_to_codec_nbt(id: BlockStateId) -> NbtTag {
    let block = Block::from_state_id(id);
    let name = block_state_name(block);
    // BlockState.CODEC writes default states as the left (block ID string) alternative.
    if id == block.default_state.id {
        return NbtTag::String(name.into());
    }

    // StateHolder.codec dispatches FULL_CODEC on id, with properties for non-singleton states.
    let mut compound = NbtCompound::new();
    compound.put_string("id", name);
    if let Some(properties) = block_state_properties_to_nbt(block, id) {
        compound.put_compound("properties", properties);
    }
    NbtTag::Compound(compound)
}

/// Decodes vanilla's `BlockState.CODEC`, returning air for invalid or unknown blocks.
#[must_use]
pub fn block_state_from_codec_nbt(tag: &NbtTag) -> BlockStateId {
    let (name, properties) = match tag {
        NbtTag::String(name) => (name.as_ref(), None),
        NbtTag::Compound(compound) => {
            let Some(name) = compound.get_string("id") else {
                return BlockStateId::AIR;
            };
            (name, compound.get_compound("properties"))
        }
        _ => return BlockStateId::AIR,
    };
    let Some(block) = Block::from_name(name) else {
        return BlockStateId::AIR;
    };
    let Some(properties) = properties else {
        return block.default_state.id;
    };
    let Some(default_properties) = block.properties(block.default_state.id) else {
        return block.default_state.id;
    };

    // StateDefinition.appendPropertyCodec defaults each missing or invalid property.
    let mut values = default_properties.to_props();
    for index in 0..values.len() {
        let (key, default_value) = values[index];
        if let Some(value) = properties.get_string(key) {
            values[index].1 = value;
            if block.state_from_properties(&values).is_none() {
                values[index].1 = default_value;
            }
        }
    }
    block
        .state_from_properties(&values)
        .map_or(block.default_state.id, |state| state.id)
}

fn block_state_name(block: &Block) -> String {
    if block.name.starts_with("minecraft:") {
        block.name.to_string()
    } else {
        format!("minecraft:{}", block.name)
    }
}

fn block_state_properties_to_nbt(block: &Block, id: BlockStateId) -> Option<NbtCompound> {
    let properties = block.properties(id)?.to_props();
    if properties.is_empty() {
        return None;
    }
    let mut compound = NbtCompound::new();
    for (key, value) in properties {
        compound.put_string(key, value.to_string());
    }
    Some(compound)
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct BlockStateCodec {
    /// Block name
    #[serde(
        deserialize_with = "parse_block_name",
        serialize_with = "block_to_string"
    )]
    pub name: &'static Block,
    /// Key-value pairs of properties
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, String>>,
}

fn parse_block_name<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<&'static Block, D::Error> {
    let s = String::deserialize(deserializer)?;
    let block =
        Block::from_name(s.as_str()).ok_or(serde::de::Error::custom("Invalid block name"))?;
    Ok(block)
}

fn block_to_string<S: Serializer>(block: &'static Block, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(block.name)
}

impl BlockStateCodec {
    #[must_use]
    pub fn get_state(&self) -> &'static BlockState {
        let state_id = self.get_state_id();
        BlockState::from_id(state_id)
    }

    #[must_use]
    pub const fn get_block(&self) -> &'static Block {
        self.name
    }

    /// Prefer this over `get_state` when the only the state ID is needed
    #[must_use]
    pub fn get_state_id(&self) -> BlockStateId {
        let block = self.name;

        let Some(properties_map) = &self.properties else {
            return block.default_state.id;
        };

        let props_iter = properties_map
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<(&str, &str)>>();

        let block_properties = block.from_properties(&props_iter);
        block_properties.to_state_id(block)
    }
}

#[cfg(test)]
mod test {
    use pumpkin_data::BlockStateId;

    use crate::chunk::palette::BLOCK_NETWORK_MAX_BITS;

    #[test]
    fn proper_network_bits_per_entry() {
        let addressable = 1u32 << BLOCK_NETWORK_MAX_BITS;
        assert!(
            u32::from(BlockStateId::COUNT) <= addressable,
            "We need to update our constants! {} states do not fit in {BLOCK_NETWORK_MAX_BITS} bits",
            BlockStateId::COUNT
        );
    }
}
