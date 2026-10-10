use super::{DragonFight, EndCrystalEntity, EndSpike, EntityBase, World};
use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

pub(super) enum RespawnAction {
    RegenerateSpike(EndSpike),
    DestroyRespawnCrystals(Vec<Arc<dyn EntityBase>>),
}

impl RespawnAction {
    // DragonRespawnStage.tick: run in the same tick, with the fight mutex released.
    pub(super) fn execute(self, world: &Arc<World>) {
        match self {
            Self::RegenerateSpike(spike) => {
                for dx in -10i32..=10 {
                    for dy in -10i32..=10 {
                        for dz in -10i32..=10 {
                            let pos = BlockPos::new(
                                spike.center_x + dx,
                                spike.height + dy,
                                spike.center_z + dz,
                            );
                            let block = world.get_block(&pos);
                            if block != &Block::BEDROCK
                                && block != &Block::OBSIDIAN
                                && block != &Block::AIR
                            {
                                world.set_block_state(
                                    &pos,
                                    Block::AIR.default_state.id,
                                    BlockFlags::NOTIFY_ALL,
                                );
                            }
                        }
                    }
                }
                world.explode(
                    Vector3::new(
                        spike.center_x as f64 + 0.5,
                        spike.height as f64,
                        spike.center_z as f64 + 0.5,
                    ),
                    5.0,
                    crate::world::ExplosionInteraction::Block,
                );
                DragonFight::regenerate_spike(world, &spike);
            }
            Self::DestroyRespawnCrystals(crystals) => {
                for entity in crystals {
                    if let Some(crystal) = entity.cast_any().downcast_ref::<EndCrystalEntity>() {
                        crystal.set_beam_target(None);
                        let explosion = crate::world::Explosion::new(
                            6.0,
                            crystal.get_entity().pos.load(),
                            world.get_block_interaction(crate::world::ExplosionInteraction::None),
                        )
                        // DragonRespawnStage retains crystals removed by earlier blasts.
                        .with_source(Some(entity.clone()));
                        world.run_explosion(&explosion);
                        crystal.get_entity().remove();
                    }
                }
            }
        }
    }
}
