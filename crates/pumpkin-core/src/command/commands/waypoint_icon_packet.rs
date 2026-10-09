use crate::entity::living::waypoint_icon::{DEFAULT, LocatorBarIcon};
use pumpkin_data::packet::clientbound::play::WAYPOINT;
use pumpkin_protocol::java::client::play::WaypointOperation;
use pumpkin_protocol::ser::{NetworkWriteExt, WritingError};
use pumpkin_protocol::{ClientPacket, MultiVersionJavaPacket, VarInt};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::version::JavaMinecraftVersion;
use std::io::Write;
use uuid::Uuid;

// The command refresh uses vanilla 26.3's codec; the general CWaypoint encoder differs.
pub(super) struct IconRefresh<'a> {
    operation: WaypointOperation,
    identifier: Uuid,
    icon: Option<&'a LocatorBarIcon>,
    position: Option<BlockPos>,
}

impl MultiVersionJavaPacket for IconRefresh<'_> {
    fn to_id(version: JavaMinecraftVersion) -> i32 {
        WAYPOINT.to_id(version)
    }
}

impl<'a> IconRefresh<'a> {
    pub(super) const fn remove(identifier: Uuid) -> Self {
        Self {
            operation: WaypointOperation::Untrack,
            identifier,
            icon: None,
            position: None,
        }
    }

    pub(super) const fn add(
        identifier: Uuid,
        icon: &'a LocatorBarIcon,
        position: BlockPos,
    ) -> Self {
        Self {
            operation: WaypointOperation::Track,
            identifier,
            icon: Some(icon),
            position: Some(position),
        }
    }
}

impl ClientPacket for IconRefresh<'_> {
    fn write_packet_data(
        &self,
        mut write: impl Write,
        _version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        // ClientboundTrackedWaypointPacket.STREAM_CODEC and TrackedWaypoint.write.
        write.write_var_int(&VarInt(self.operation as i32))?;
        write.write_bool(true)?; // FriendlyByteBuf.writeEither selects the UUID branch.
        write.write_uuid(&self.identifier)?;
        let style = self.icon.and_then(|icon| icon.style.as_deref());
        write.write_string(style.unwrap_or(DEFAULT))?;
        let color = self.icon.and_then(|icon| icon.color);
        write.write_bool(color.is_some())?;
        if let Some(color) = color {
            // Waypoint.Icon.STREAM_CODEC uses ByteBufCodecs.RGB_COLOR's three bytes.
            write.write_all(&color.to_be_bytes()[1..])?;
        }
        // TrackedWaypoint.Type: EMPTY = 0, VEC3I = 1.
        write.write_var_int(&VarInt(i32::from(self.position.is_some())))?;
        if let Some(position) = self.position {
            // TrackedWaypoint.Vec3iWaypoint.writeContents uses three VarInts.
            for coordinate in [position.0.x, position.0.y, position.0.z] {
                write.write_var_int(&VarInt(coordinate))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review3_waypoint_refresh_codec_matches_vanilla_bytes() -> Result<(), WritingError> {
        let version = pumpkin_data::packet::CURRENT_MC_VERSION;
        let mut bytes = Vec::new();
        IconRefresh::remove(Uuid::nil()).write_packet_data(&mut bytes, &version)?;
        // Operation, Either.left(UUID), Waypoint.Icon.NULL, then Type.EMPTY.
        let mut expected = vec![1, 1];
        expected.extend_from_slice(&[0; 16]);
        expected.extend_from_slice(b"\x11minecraft:default\x00\x00");
        assert_eq!(bytes, expected);

        let mut icon = LocatorBarIcon {
            style: Some("minecraft:bowtie".into()),
            color: None,
        };
        bytes.clear();
        IconRefresh::add(Uuid::nil(), &icon, BlockPos::new(-1, 64, 2))
            .write_packet_data(&mut bytes, &version)?;
        expected = vec![0, 1];
        expected.extend_from_slice(&[0; 16]);
        // Optional color absent, Type.VEC3I, then three VarInts (including negative X).
        expected.extend_from_slice(b"\x10minecraft:bowtie\x00\x01\xff\xff\xff\xff\x0f\x40\x02");
        assert_eq!(bytes, expected);

        icon.color = Some(0xabcdef);
        bytes.clear();
        IconRefresh::add(Uuid::nil(), &icon, BlockPos::new(-1, 64, 2))
            .write_packet_data(&mut bytes, &version)?;
        expected = vec![0, 1];
        expected.extend_from_slice(&[0; 16]);
        expected.extend_from_slice(
            b"\x10minecraft:bowtie\x01\xab\xcd\xef\x01\xff\xff\xff\xff\x0f\x40\x02",
        );
        assert_eq!(bytes, expected);
        Ok(())
    }
}
