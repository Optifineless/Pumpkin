use pumpkin_data::packet::serverbound::play::SET_GAME_RULE;
use pumpkin_macros::java_packet;

use crate::{
    ServerPacket,
    codec::var_int::VarInt,
    ser::{NetworkReadExt, NetworkReadSliceExt, ReadingError},
};
use pumpkin_util::version::JavaMinecraftVersion;

pub struct GameRuleEntry<'a> {
    pub game_rule_key: &'a str,
    pub value: &'a str,
}

#[java_packet(SET_GAME_RULE)]
pub struct SSetGameRule<'a> {
    pub entries: Vec<GameRuleEntry<'a>>,
}

impl<'a> ServerPacket<'a> for SSetGameRule<'a> {
    fn read(bytebuf: &mut &'a [u8], _version: &JavaMinecraftVersion) -> Result<Self, ReadingError> {
        let count = bytebuf.get_var_int()?.0 as usize;
        // ServerboundSetGameRulePacket uses ByteBufCodecs.list without a count limit.
        // Cap initial capacity by the registry; duplicate entries remain legal.
        let capacity = crate::ser::collection_capacity(count)?
            .min(pumpkin_data::game_rules::GameRule::all().len());
        let mut entries = Vec::with_capacity(capacity);
        for _ in 0..count {
            let game_rule_key = bytebuf.get_str_borrowed()?;
            let value = bytebuf.get_str_borrowed()?;
            entries.push(GameRuleEntry {
                game_rule_key,
                value,
            });
        }
        Ok(Self { entries })
    }
}

impl crate::ClientPacket for SSetGameRule<'_> {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        use crate::ser::NetworkWriteExt;
        write.write_var_int(&VarInt(self.entries.len() as i32))?;
        for entry in &self.entries {
            write.write_string(entry.game_rule_key)?;
            write.write_string(entry.value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ser::NetworkWriteExt;

    #[test]
    fn giant_game_rule_count_without_entries_reports_eof() {
        // VarInt -1 must fail before the old usize cast can overflow Vec's capacity.
        let mut negative: &[u8] = &[0xff, 0xff, 0xff, 0xff, 0x0f];
        assert!(matches!(
            SSetGameRule::read(&mut negative, &JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
        let mut bytes: &[u8] = &[0xff, 0xff, 0xff, 0xff, 0x07];
        assert!(matches!(
            SSetGameRule::read(&mut bytes, &JavaMinecraftVersion::V_26_3),
            Err(ReadingError::CleanEOF(_) | ReadingError::Incomplete(_))
        ));
    }

    #[test]
    fn duplicate_game_rules_can_exceed_the_registry_size() {
        let count = pumpkin_data::game_rules::GameRule::all().len() + 1;
        let mut bytes = Vec::new();
        bytes.write_var_int(&VarInt(count as i32)).unwrap();
        for _ in 0..count {
            bytes.extend_from_slice(&[1, b'a', 1, b'b']);
        }
        let packet =
            SSetGameRule::read(&mut bytes.as_slice(), &JavaMinecraftVersion::V_26_3).unwrap();
        assert_eq!(packet.entries.len(), count);
    }
}
