use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::position::BlockPos;

use crate::entity::player::Player;

use super::World;

impl World {
    /// Mirrors 26.3 `DedicatedServer.isUnderSpawnProtection` for Pumpkin's shared global spawn.
    #[must_use]
    pub fn is_in_spawn_protection(&self, player: &Player, position: &BlockPos) -> bool {
        let Some(server) = self.server.upgrade() else {
            return false;
        };

        // LevelData persists the shared global spawn as minecraft:overworld.
        if self.dimension.minecraft_name != Dimension::OVERWORLD.minecraft_name {
            return false;
        }

        let (operators_exist, is_operator) = {
            let operators = server
                .data
                .operator_config
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                !operators.ops.is_empty(),
                operators.get_entry(&player.gameprofile.id).is_some(),
            )
        };
        if !operators_exist || is_operator {
            return false;
        }

        let radius = server.basic_config.spawn_protection;
        if radius == 0 {
            return false;
        }

        let radius = u32::try_from(i32::try_from(radius).unwrap_or(i32::MAX)).unwrap_or(u32::MAX);
        let spawn = self.get_spawn_location().0;
        position.0.x.abs_diff(spawn.0.x) <= radius && position.0.z.abs_diff(spawn.0.z) <= radius
    }
}
