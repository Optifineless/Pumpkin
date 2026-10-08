use crate::{
    ClientPacket,
    ser::{NetworkWriteExt, WritingError},
};
use pumpkin_data::packet::clientbound::config::KEEP_ALIVE;
use pumpkin_macros::java_packet;
use pumpkin_util::version::JavaMinecraftVersion;

// ClientboundKeepAlivePacket.STREAM_CODEC, shared by configuration and play.
#[java_packet(KEEP_ALIVE)]
pub struct CConfigKeepAlive {
    pub keep_alive_id: i64,
}

impl ClientPacket for CConfigKeepAlive {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        write.write_i64_be(self.keep_alive_id)
    }
}
