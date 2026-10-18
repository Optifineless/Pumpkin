use super::{DataComponentCodec, deserialize_idset, serialize_idset};
use crate::ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError};
use pumpkin_data::{
    data_component_impl::{IDSet, MobVisibilityImpl},
    entity::EntityType,
    tag::Taggable,
};
use pumpkin_util::identifier::Identifier;
use std::borrow::Cow;

impl DataComponentCodec<Self> for MobVisibilityImpl {
    // MobVisibility.STREAM_CODEC carries its HolderSet before an unvalidated float.
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        serialize_idset(&self.targeting_entity_types, seq)?;
        seq.write_f32(self.visibility)
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let mut targeting_entity_types = deserialize_idset(seq)?;
        if let IDSet::Tag(tag) = &mut targeting_entity_types {
            // ByteBufCodecs.holderSet resolves the Identifier against the live registry.
            let name = Identifier::parse(tag)
                .map_err(|_| ReadingError::Message("Invalid mob visibility entity tag".into()))?
                .to_string();
            EntityType::get_tag_values(&name)
                .ok_or_else(|| ReadingError::Message("Invalid mob visibility entity tag".into()))?;
            *tag = Cow::Owned(name);
        }
        Ok(Self {
            targeting_entity_types,
            visibility: seq.get_f32()?,
        })
    }
}
