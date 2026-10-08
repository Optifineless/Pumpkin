use pumpkin_data::packet::serverbound::play::CHAT_COMMAND_SIGNED;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

use crate::{
    ClientPacket, ServerPacket,
    codec::var_int::VarInt,
    ser::{NetworkReadExt, NetworkReadSliceExt, NetworkWriteExt, ReadingError, WritingError},
};

pub struct ArgumentSignature<'a> {
    pub name: &'a str,
    pub signature: &'a [u8],
}

#[java_packet(CHAT_COMMAND_SIGNED)]
pub struct SChatCommandSigned<'a> {
    pub command: &'a str,
    pub timestamp: i64,
    pub salt: i64,
    pub argument_signatures: Vec<ArgumentSignature<'a>>,
    pub message_count: VarInt,
    pub acknowledged: &'a [u8],
    pub checksum: u8,
}

impl<'a> ServerPacket<'a> for SChatCommandSigned<'a> {
    fn read(read: &mut &'a [u8], version: &JavaMinecraftVersion) -> Result<Self, ReadingError> {
        // ArgumentSignatures.STREAM_CODEC bounds the list to eight entries.
        const MAX_ARGUMENT_COUNT: usize = 8;
        let command = read.get_str_bounded_borrowed(256)?;
        let timestamp = read.get_i64_be()?;
        let salt = read.get_i64_be()?;
        let arg_count = read.get_var_int()?.0 as usize;
        if arg_count > MAX_ARGUMENT_COUNT {
            return Err(ReadingError::TooLarge("Argument signatures".into()));
        }
        let mut argument_signatures = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            let name = read.get_str_bounded_borrowed(16)?;
            let signature = read.read_slice_borrowed(256)?;
            argument_signatures.push(ArgumentSignature { name, signature });
        }
        let message_count = read.get_var_int()?;
        let acknowledged = read.read_slice_borrowed(3)?;
        let checksum = if *version >= JavaMinecraftVersion::V_1_21_5 {
            read.get_u8()?
        } else {
            0
        };

        Ok(Self {
            command,
            timestamp,
            salt,
            argument_signatures,
            message_count,
            acknowledged,
            checksum,
        })
    }
}

impl ClientPacket for SChatCommandSigned<'_> {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        write.write_string(self.command)?;
        write.write_i64_be(self.timestamp)?;
        write.write_i64_be(self.salt)?;
        write.write_var_int(&VarInt(self.argument_signatures.len() as i32))?;
        for arg in &self.argument_signatures {
            write.write_string(arg.name)?;
            write.write_slice(arg.signature)?;
        }
        write.write_var_int(&self.message_count)?;
        write.write_slice(self.acknowledged)?;
        if *version >= JavaMinecraftVersion::V_1_21_5 {
            write.write_u8(self.checksum)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_signature_count_is_bounded_before_allocation() {
        let mut eight = vec![0; 17]; // Empty command, timestamp and salt.
        eight.push(8);
        for _ in 0..8 {
            eight.push(0); // Empty argument name followed by the fixed signature.
            eight.extend_from_slice(&[0; 256]);
        }
        eight.extend_from_slice(&[0; 5]); // Offset, twenty acknowledgement bits, checksum.
        assert!(
            SChatCommandSigned::read(&mut eight.as_slice(), &JavaMinecraftVersion::V_26_3).is_ok()
        );
        let mut nine = [0; 18];
        nine[17] = 9;
        assert!(matches!(
            SChatCommandSigned::read(&mut nine.as_slice(), &JavaMinecraftVersion::V_26_3),
            Err(ReadingError::TooLarge(_))
        ));
    }
}
