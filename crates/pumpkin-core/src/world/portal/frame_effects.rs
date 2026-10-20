use std::sync::Arc;

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::{BedPart, DoubleBlockHalf, OakDoorLikeProperties, WhiteBedLikeProperties},
    tag::Taggable,
};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::block::entities::{BlockEntity, block_entity_from_nbt, block_entity_name};
use crate::world::{
    BlockEntityMutationPermit, World, guard_effects, neighbor_updater::UPDATE_LIMIT,
};

/// Captures positional cleanup targets before deferred callbacks can replace them.
pub(in crate::world) struct RetiredBlockEffects {
    partner: Option<RetiredPartner>,
    suppress_bed_partner_shape: bool,
}

struct RetiredPartner {
    position: BlockPos,
    state: BlockStateId,
}

impl RetiredBlockEffects {
    pub(in crate::world) fn capture(world: &World, position: &BlockPos, old: BlockStateId) -> Self {
        let block = Block::from_state_id(old);
        // AbstractBedBlock.getConnectedDirection and DoorBlock/DoublePlantBlock's
        // DoubleBlockHalf lookup, at LevelChunk.setBlockState's removal boundary.
        let partner = if block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEDS)
            || block == &Block::STRAW_BED
        {
            let props = WhiteBedLikeProperties::from_state_id(old);
            let direction = if props.part == BedPart::Head {
                props.facing.opposite()
            } else {
                props.facing
            };
            Some(position.offset(direction.to_offset()))
        } else if block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_DOORS) {
            Some(match OakDoorLikeProperties::from_state_id(old).half {
                DoubleBlockHalf::Upper => position.down(),
                DoubleBlockHalf::Lower => position.up(),
            })
        } else {
            None
        };
        Self {
            suppress_bed_partner_shape: false,
            partner: partner.map(|position| RetiredPartner {
                state: world.get_block_state_id(&position),
                position,
            }),
        }
    }

    fn partner_is_unchanged(&self, world: &World) -> bool {
        self.partner
            .as_ref()
            .is_none_or(|partner| world.get_block_state_id(&partner.position) == partner.state)
    }

    pub(in crate::world) const fn suppress_bed_partner_shape(&mut self) {
        self.suppress_bed_partner_shape = true;
    }

    pub(in crate::world) fn bed_partner_to_skip(
        &self,
        world: &World,
        old: BlockStateId,
    ) -> Option<BlockPos> {
        let block = Block::from_state_id(old);
        if (block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEDS) || block == &Block::STRAW_BED)
            && (self.suppress_bed_partner_shape || !self.partner_is_unchanged(world))
        {
            return self.partner.as_ref().map(|partner| partner.position);
        }
        None
    }

    pub(in crate::world) fn orphaned_bed_partner(
        &self,
        world: &World,
        position: &BlockPos,
        old: BlockStateId,
    ) -> Option<(BlockPos, BlockStateId)> {
        let block = Block::from_state_id(old);
        let partner = self.partner.as_ref()?;
        if !(block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEDS) || block == &Block::STRAW_BED)
            || Block::from_state_id(partner.state) != block
            || !self.partner_is_unchanged(world)
        {
            return None;
        }
        let part = WhiteBedLikeProperties::from_state_id(partner.state).part;
        if WhiteBedLikeProperties::from_state_id(old).part == part {
            return None;
        }
        let replacement = world.get_block_state_id(position);
        if Block::from_state_id(replacement) == block
            && WhiteBedLikeProperties::from_state_id(replacement).part != part
        {
            return None;
        }
        Some((partner.position, partner.state))
    }

    /// Delivers pair notifications when a newer block occupies the retired position.
    pub(in crate::world) fn notify_replaced_pair(
        &self,
        world: &Arc<World>,
        position: &BlockPos,
        old: BlockStateId,
        flags: BlockFlags,
    ) {
        // Level.setBlock's neighbour notification survives skipped pair teardown,
        // even when the plugin also replaced the retired position during replay.
        if !self.partner_is_unchanged(world) && flags.contains(BlockFlags::NOTIFY_NEIGHBORS) {
            world.update_neighbors_at(position, Block::from_state_id(old), None);
        }
    }
}

/// Retains removed block-entity state for callbacks without republishing its cache entry.
pub(in crate::world) struct RetiredBlockEntity {
    pub(in crate::world) live: Option<Arc<dyn BlockEntity>>,
    pub(in crate::world) pending: Option<NbtCompound>,
}

impl RetiredBlockEntity {
    pub(in crate::world) fn on_block_replaced(self, world: &Arc<World>, position: &BlockPos) {
        // LevelChunk.setBlockState/preRemoveSideEffects: use the retired instance,
        // never a replacement installed by an earlier callback in this batch.
        if let Some(entity) = self
            .live
            .or_else(|| self.pending.as_ref().and_then(block_entity_from_nbt))
        {
            world.bind_block_entity_context(entity.as_ref());
            entity.on_block_replaced(world, position);
        }
    }
}

// PortalForcer.createPortal writes the complete frame before creation ownership is
// released. Retain Level.setBlock's post-write effects, but dispatch outside that owner.
pub(super) fn frame_mutation_permits(
    world: &Arc<World>,
    lower_corner: BlockPos,
) -> Option<Vec<BlockEntityMutationPermit<'_>>> {
    let mut permits = Vec::new();
    // PortalForcer frames/fallback clearing and EndPlatformFeature.createEndPlatform
    // share this horizontal footprint; reserve it before either starts writing.
    for chunk_x in ((lower_corner.0.x - 2) >> 4)..=((lower_corner.0.x + 2) >> 4) {
        for chunk_z in ((lower_corner.0.z - 2) >> 4)..=((lower_corner.0.z + 2) >> 4) {
            let position = BlockPos::new(chunk_x << 4, lower_corner.0.y, chunk_z << 4);
            permits.push(world.block_entity_mutation_permit(position)?);
        }
    }
    Some(permits)
}

pub(super) fn set_frame_block(
    world: &Arc<World>,
    position: &BlockPos,
    state: BlockStateId,
    flags: BlockFlags,
) {
    if !guard_effects::is_active() {
        world.set_block_state(position, state, flags);
        return;
    }
    if !world.is_in_build_limit(*position) {
        return;
    }
    let Some(_mutation) = world.block_entity_mutation_permit(*position) else {
        tracing::warn!("Portal frame update rejected while chunk is unloading at {position:?}");
        return;
    };
    let old = world
        .write_block_state_if(position, state, |_| true)
        .unwrap_or(Block::AIR.default_state.id);
    let retired_effects = RetiredBlockEffects::capture(world, position, old);
    // Retire live and saved caches under their existing publication gate. Constructing
    // pending entities or binding native context must wait until creation unlocks.
    let retired = (Block::from_state_id(old) != Block::from_state_id(state)
        && block_entity_name(Block::from_state_id(old)).is_some())
    .then(|| world.remove_loaded_block_entity(position));
    let world = world.clone();
    let position = *position;
    let effect = move || {
        if let Some(retired) = retired
            && !flags.contains(BlockFlags::SKIP_BLOCK_ENTITY_REPLACED_CALLBACK)
        {
            retired.on_block_replaced(&world, &position);
        }
        world.on_block_state_set_with_block_entity(
            &position,
            old,
            state,
            flags,
            UPDATE_LIMIT,
            Some(&retired_effects),
        );
    };
    if let Err(effect) = guard_effects::defer(effect) {
        effect();
    }
}

// LevelChunk.setBlockState retires block entities before affectNeighborsAfterRemoval.
// A sign's callback only repeats that retirement; deferred delivery owns no current
// entity, so it must not remove anything published after the captured retirement.
pub(in crate::world) fn dispatch_old_block_callback(
    world: &Arc<World>,
    position: &BlockPos,
    old: BlockStateId,
    moved: bool,
    retired: Option<&RetiredBlockEffects>,
) {
    let block = Block::from_state_id(old);
    if let Some(retired) = retired {
        // LevelChunk.setBlockState removes synchronously; a deferred callback must
        // not tear down a different pair installed by an earlier physics event.
        if !retired.partner_is_unchanged(world)
            || block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_ALL_SIGNS)
        {
            return;
        }
    }
    // ServerLevel.updatePOIOnBlockStateChange: preserve the current portal registration.
    if block == &Block::NETHER_PORTAL && world.get_block(position) == &Block::NETHER_PORTAL {
        return;
    }
    world
        .block_registry
        .on_state_replaced(world, block, position, old, moved);
}
