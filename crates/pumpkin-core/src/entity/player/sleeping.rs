use std::sync::{Arc, atomic::Ordering::Relaxed};

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{BedPart, WhiteBedLikeProperties},
    entity::EntityPose,
    tag::{self, Taggable},
    tracked_data,
};
use pumpkin_protocol::java::client::play::{Animation, CEntityAnimation};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use super::{Player, statistics};
use crate::{
    block::blocks::straw_bed::StrawBedBlock, entity::EntityBase,
    plugin::api::events::player::player_bed::PlayerBedLeaveEvent, world::World,
};

struct WakeContext {
    operation: u64,
    world: Arc<World>,
    bed_pos: Option<BlockPos>,
    life: u64,
    registered: bool,
}

impl WakeContext {
    fn is_current(&self, player: &Player) -> bool {
        player.sleep_operation.load(Relaxed) == self.operation
            && player.sleeping_bed_pos.load() == self.bed_pos
            && player.living_entity.damage_lifecycle() == self.life
            && Arc::ptr_eq(&player.world(), &self.world)
            && self
                .world
                .get_player_by_uuid(player.gameprofile.id)
                .map_or(!self.registered, |current| {
                    std::ptr::eq(Arc::as_ptr(&current), player)
                })
    }
}

impl Player {
    pub(super) fn invalidate_sleep_operation(&self) {
        // Even revisions admit a wake; odd revisions reserve one without holding a lock.
        let _ = self
            .sleep_operation
            .fetch_update(Relaxed, Relaxed, |old| Some(old.wrapping_add(2) & !1));
    }

    pub(super) fn tick_sleeping(&self) {
        // LivingEntity.tick / checkBedExists: validate the remembered bed before advancing sleep.
        let revision = self.sleep_operation.load(Relaxed);
        if let Some(position) = self.sleeping_bed_pos.load() {
            let bed = self.world().get_block(&position);
            if bed != &Block::STRAW_BED && !bed.has_tag(&tag::Block::MINECRAFT_BEDS) {
                self.stop_sleeping_at_revision(true, revision);
            }
        }
        if let Some(timer) = self.sleeping_since.load()
            && timer < 101
        {
            self.sleeping_since.store(Some(timer + 1));
        }
    }

    pub(super) fn stop_sleeping(&self, removed_bed: bool) {
        let revision = self.sleep_operation.load(Relaxed);
        self.stop_sleeping_at_revision(removed_bed, revision);
    }

    fn stop_sleeping_at_revision(&self, removed_bed: bool, revision: u64) {
        if revision & 1 != 0
            || self
                .sleep_operation
                .compare_exchange(revision, revision.wrapping_add(1), Relaxed, Relaxed)
                .is_err()
        {
            return;
        }
        let world = self.world();
        let context = WakeContext {
            operation: revision.wrapping_add(1),
            registered: world.get_player_by_uuid(self.gameprofile.id).is_some(),
            world,
            bed_pos: self.sleeping_bed_pos.load(),
            life: self.living_entity.damage_lifecycle(),
        };
        self.finish_stop_sleeping(&context, removed_bed);
        let _ = self.sleep_operation.compare_exchange(
            context.operation,
            context.operation.wrapping_add(1),
            Relaxed,
            Relaxed,
        );
    }

    fn finish_stop_sleeping(&self, context: &WakeContext, removed_bed: bool) {
        if !context.is_current(self) {
            return;
        }
        let entity = self.get_entity();
        let Some(bed_pos) = context.bed_pos else {
            entity.set_pose_if(EntityPose::Standing, || context.is_current(self));
            if context.is_current(self) && entity.pose.load() == EntityPose::Standing {
                self.sleeping_since.store(None);
            }
            return;
        };
        let bed_state = context.world.get_block_state_id_if_loaded(&bed_pos);
        let partner = bed_state.and_then(|state| bed_partner(&context.world, bed_pos, state));
        if let Some(server) = context.world.server.upgrade()
            && let Some(player) = context.world.get_player_by_uuid(self.gameprofile.id)
        {
            let mut event = PlayerBedLeaveEvent::new(player, bed_pos);
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if !context.is_current(self) {
            return;
        }
        // A missing bed must never acquire cleanup rights over a callback's replacement.
        if !removed_bed
            && let Some(state) = bed_state
            && context.world.get_block_state_id_if_loaded(&bed_pos) == Some(state)
        {
            let bed = Block::from_state_id(state);
            if bed == &Block::STRAW_BED {
                StrawBedBlock::destroy_after_use(&context.world, bed_pos);
            } else if bed.has_tag(&tag::Block::MINECRAFT_BEDS) {
                clear_occupied(&context.world, bed_pos, state);
                if context.is_current(self)
                    && let Some((position, expected)) = partner
                {
                    clear_occupied(&context.world, position, expected);
                }
            }
        }
        if !context.is_current(self) {
            return;
        }
        // LivingEntity.stopSleeping keeps position/rotation when the bed no longer exists.
        entity.set_pose_if(EntityPose::Standing, || context.is_current(self));
        if !context.is_current(self) || entity.pose.load() != EntityPose::Standing {
            return;
        }
        entity.set_pos(self.position());
        entity.set_synced_data(tracked_data::player::SLEEPING_POS_ID, None::<BlockPos>);
        self.set_stat(
            statistics::StatisticCategory::Custom,
            statistics::CustomStatistic::TimeSinceRest as i32,
            0,
        );
        self.sleeping_since.store(None);
        self.sleeping_bed_pos.store(None);
        context.world.broadcast_to_chunk(
            entity.chunk_pos.load(),
            &CEntityAnimation::new(self.entity_id().into(), Animation::LeaveBed),
        );
    }
}

fn bed_partner(
    world: &World,
    position: BlockPos,
    state: BlockStateId,
) -> Option<(BlockPos, BlockStateId)> {
    let bed = Block::from_state_id(state);
    if !bed.has_tag(&tag::Block::MINECRAFT_BEDS) {
        return None;
    }
    let properties = WhiteBedLikeProperties::from_state_id(state);
    let offset = if properties.part == BedPart::Head {
        properties.facing.opposite()
    } else {
        properties.facing
    };
    let partner = position.offset(offset.to_offset());
    let expected = world.get_block_state_id_if_loaded(&partner)?;
    if Block::from_state_id(expected) != bed {
        return None;
    }
    let other = WhiteBedLikeProperties::from_state_id(expected);
    (other.part != properties.part && other.facing == properties.facing)
        .then_some((partner, expected))
}

fn clear_occupied(world: &Arc<World>, position: BlockPos, expected: BlockStateId) {
    let bed = Block::from_state_id(expected);
    let mut properties = WhiteBedLikeProperties::from_state_id(expected);
    properties.occupied = false;
    let _ = world.set_block_state_if(
        &position,
        properties.to_state_id(bed),
        BlockFlags::NOTIFY_LISTENERS,
        |current| current == expected,
    );
}

#[cfg(test)]
mod tests;
