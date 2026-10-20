use super::DataComponentCodec;
use crate::{
    VarInt,
    ser::{NetworkReadExt, NetworkWriteExt, ReadingError, WritingError},
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{DataComponentImpl, JukeboxPlayableImpl},
    registry::{REGISTRY_V_26_3, StaticRegistryEntry},
};

fn song_registry() -> &'static [StaticRegistryEntry] {
    // Use the same order as Registry::get_synced sends during configuration.
    REGISTRY_V_26_3
        .iter()
        .find(|registry| registry.registry_id == "jukebox_song")
        .map_or(&[], |registry| registry.entries)
}

fn song_holder(song: &str) -> Option<i32> {
    let name = song.strip_prefix("minecraft:").unwrap_or(song);
    song_registry()
        .iter()
        .position(|entry| entry.name == name)
        .and_then(|index| i32::try_from(index + 1).ok())
}

pub(in crate::codec) fn can_serialize_component(
    id: DataComponent,
    data: &dyn DataComponentImpl,
) -> bool {
    // Legacy stopped references remain in saves, but have no network holder.
    id != DataComponent::JukeboxPlayable
        || data
            .as_any()
            .downcast_ref::<JukeboxPlayableImpl>()
            .is_some_and(|playable| song_holder(playable.song).is_some())
}

impl DataComponentCodec<Self> for JukeboxPlayableImpl {
    fn serialize(&self, seq: &mut impl NetworkWriteExt) -> Result<(), WritingError> {
        // JukeboxPlayable/JukeboxSong.STREAM_CODEC -> ByteBufCodecs.holder.
        let holder = song_holder(self.song)
            .ok_or_else(|| WritingError::Message("Unknown jukebox song reference".into()))?;
        seq.write_var_int(&VarInt(holder))
    }

    fn deserialize(seq: &mut impl NetworkReadExt) -> Result<Self, ReadingError> {
        let holder = seq.get_var_int()?.0;
        let song = usize::try_from(holder)
            .ok()
            .and_then(|holder| holder.checked_sub(1))
            .and_then(|index| song_registry().get(index))
            .ok_or_else(|| {
                ReadingError::Message("Unsupported or unknown jukebox song holder".into())
            })?;
        Ok(Self { song: song.name })
    }
}

#[cfg(test)]
#[path = "jukebox_playable_tests.rs"]
mod tests;
