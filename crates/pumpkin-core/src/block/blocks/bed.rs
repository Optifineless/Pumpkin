use std::sync::Arc;

use crate::block::entities::bed::BedBlockEntity;
use pumpkin_data::block_properties::BedPart;
use pumpkin_data::entity::EntityType;
use pumpkin_data::translation;
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::block::OnLandedUponArgs;
use crate::block::UpdateEntityMovementAfterFallOnArgs;
use crate::block::bounce_entity_after_fall;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, CanPlaceAtArgs, GetStateForNeighborUpdateArgs, NormalUseArgs, OnPlaceArgs,
    PathComputationType, PlacedArgs, PlayerWillDestroyArgs,
};
use crate::entity::{Entity, EntityBase, player::Player};
use crate::world::World;

type BedProperties = pumpkin_data::block_properties::WhiteBedLikeProperties;

const NO_SLEEP_IDS: &[u16] = &[
    EntityType::BLAZE.id,
    EntityType::BOGGED.id,
    EntityType::SKELETON.id,
    EntityType::STRAY.id,
    EntityType::WITHER_SKELETON.id,
    EntityType::BREEZE.id,
    EntityType::CREAKING.id,
    EntityType::CREEPER.id,
    EntityType::DROWNED.id,
    EntityType::ENDERMITE.id,
    EntityType::EVOKER.id,
    EntityType::GIANT.id,
    EntityType::GUARDIAN.id,
    EntityType::ELDER_GUARDIAN.id,
    EntityType::ILLUSIONER.id,
    EntityType::OCELOT.id,
    EntityType::PIGLIN.id,
    EntityType::PIGLIN_BRUTE.id,
    EntityType::PILLAGER.id,
    EntityType::PHANTOM.id,
    EntityType::RAVAGER.id,
    EntityType::SILVERFISH.id,
    EntityType::SPIDER.id,
    EntityType::CAVE_SPIDER.id,
    EntityType::VEX.id,
    EntityType::VINDICATOR.id,
    EntityType::WARDEN.id,
    EntityType::WITCH.id,
    EntityType::WITHER.id,
    EntityType::ZOGLIN.id,
    EntityType::ZOMBIE.id,
    EntityType::ZOMBIE_VILLAGER.id,
    EntityType::HUSK.id,
    EntityType::ENDERMAN.id,
    EntityType::ZOMBIFIED_PIGLIN.id,
];

#[pumpkin_block_from_tag("minecraft:beds")]
pub struct BedBlock;

impl BlockBehaviour for BedBlock {
    fn can_place_at(&self, args: CanPlaceAtArgs<'_>) -> bool {
        if let Some(player) = args.player {
            let facing = player.get_entity().get_horizontal_facing();
            return args
                .block_accessor
                .get_block_state(args.position)
                .replaceable()
                && args
                    .block_accessor
                    .get_block_state(&args.position.offset(facing.to_offset()))
                    .replaceable();
        }
        false
    }

    fn on_landed_upon(&self, args: OnLandedUponArgs<'_>) {
        if let Some(living) = args.entity.get_living_entity() {
            living.handle_fall_damage(args.entity, args.fall_distance * 0.5, 1.0);
        }
    }

    fn update_entity_movement_after_fall_on(&self, args: UpdateEntityMovementAfterFallOnArgs<'_>) {
        bounce_entity_after_fall(args.entity, 0.66);
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut bed_props = BedProperties::default(args.block);

        bed_props.facing = args.player.get_entity().get_horizontal_facing();
        bed_props.part = BedPart::Foot;

        bed_props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let bed_entity = BedBlockEntity::new(*args.position);
            if !args
                .world
                .add_block_entity_from_observed(Arc::new(bed_entity), args.state_id)
            {
                return;
            }

            let mut bed_head_props = BedProperties::default(args.block);
            bed_head_props.facing = BedProperties::from_state_id(args.state_id).facing;
            bed_head_props.part = BedPart::Head;

            let bed_head_pos = args.position.offset(bed_head_props.facing.to_offset());
            let head_state = bed_head_props.to_state_id(args.block);
            let observed = args.world.get_block_state_id(&bed_head_pos);
            if observed != head_state
                && (!observed.to_state().replaceable()
                    || !args.world.set_block_states_from_observed(&[
                        (
                            *args.position,
                            args.state_id,
                            args.state_id,
                            BlockFlags::empty(),
                        ),
                        (
                            bed_head_pos,
                            observed,
                            head_state,
                            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
                        ),
                    ]))
            {
                return;
            }

            let bed_head_entity = BedBlockEntity::new(bed_head_pos);
            args.world
                .add_block_entity_from_observed(Arc::new(bed_head_entity), head_state);
        }
    }

    fn player_will_destroy(&self, args: PlayerWillDestroyArgs<'_>) {
        super::bed_pair::player_will_destroy(args);
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        super::bed_pair::update_shape(&args)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        Self::use_bed(args.world, args.player, args.block, args.position)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl BedBlock {
    #[expect(clippy::too_many_lines)]
    fn use_bed(
        world: &Arc<World>,
        player: &Arc<Player>,
        block: &Block,
        position: &BlockPos,
    ) -> BlockActionResult {
        let Some((bed_head_pos, bed_foot_pos, state_id)) = world.bed_pair_for_use(block, *position)
        else {
            return BlockActionResult::Consume;
        };
        let bed_props = BedProperties::from_state_id(state_id);

        // AbstractBedBlock.useWithoutItem dispatches BedBlock.destroyOnUse before sleep admission.
        if world.dimension.bed_rule.destroy_on_use {
            if let Some(message) = world.dimension.bed_rule.error_message
                && let Ok(message) = serde_json::from_str(message)
            {
                player.send_system_message_raw(&message, true);
            }
            if world.destroy_bed_on_use(bed_head_pos, bed_foot_pos, state_id) {
                world.explode_bad_respawn(bed_head_pos.to_centered_f64());
            }
            return BlockActionResult::SuccessServer;
        }

        let is_dark = world.is_dark_outside();
        let can_sleep = world.dimension.bed_rule.can_sleep(is_dark);
        let can_set_spawn = world.dimension.bed_rule.can_set_spawn(is_dark);

        if !can_set_spawn && !can_sleep {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_NO_SLEEP,
                    translation::bedrock::TILE_BED_NOSLEEP
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }

        // Make sure the bed is not obstructed
        if world.get_block_state(&bed_head_pos.up()).is_solid()
            || world.get_block_state(&bed_foot_pos.up()).is_solid()
        {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_OBSTRUCTED,
                    translation::bedrock::TILE_BED_OBSTRUCTED
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }

        // Make sure the bed is not occupied
        if bed_props.occupied {
            // TODO: Wake up villager

            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_OCCUPIED,
                    translation::bedrock::TILE_BED_OCCUPIED
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }

        // Make sure player is close enough
        if !player
            .position()
            .is_within_bounds(bed_head_pos.to_f64(), 3.0, 3.0, 3.0)
            && !player
                .position()
                .is_within_bounds(bed_foot_pos.to_f64(), 3.0, 3.0, 3.0)
        {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_TOO_FAR_AWAY,
                    translation::bedrock::TILE_BED_TOOFAR
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }

        // Set respawn point
        if can_set_spawn
            && player.set_respawn_point(
                world.dimension.clone(),
                bed_head_pos,
                player.get_entity().yaw.load(),
                player.get_entity().pitch.load(),
                false,
            )
        {
            player.send_system_message(&pumpkin_macros::translate_cross!(
                translation::java::BLOCK_MINECRAFT_SET_SPAWN,
                translation::bedrock::TILE_BED_RESPAWNSET
            ));
        }

        // Make sure the time and weather allows sleep
        if !can_sleep {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_NO_SLEEP,
                    translation::bedrock::TILE_BED_NOSLEEP
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }

        // Make sure there are no monsters nearby
        for entity in world.entities.load().iter() {
            if !entity_prevents_sleep(entity.get_entity()) {
                continue;
            }

            let pos = entity.get_entity().pos.load();
            if pos.is_within_bounds(bed_head_pos.to_f64(), 8.0, 5.0, 8.0)
                || pos.is_within_bounds(bed_foot_pos.to_f64(), 8.0, 5.0, 8.0)
            {
                player.send_system_message_raw(
                    &pumpkin_macros::translate_cross!(
                        translation::java::BLOCK_MINECRAFT_BED_NOT_SAFE,
                        translation::bedrock::TILE_BED_NOTSAFE
                    ),
                    true,
                );
                return BlockActionResult::SuccessServer;
            }
        }

        if let Some(server) = world.server.upgrade() {
            let mut event =
                crate::plugin::api::events::player::player_bed::PlayerBedEnterEvent::new(
                    player.clone(),
                    bed_head_pos,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return BlockActionResult::SuccessServer;
            }
        }

        if !Self::set_occupied(true, world, block, &bed_head_pos, state_id) {
            return BlockActionResult::SuccessServer;
        }
        player.sleep(bed_head_pos);
        player.trigger_advancement(
            crate::entity::player::advancement::trigger::AdvancementTrigger::SleptInBed,
        );
        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::SleepInBed as i32,
            1,
        );
        BlockActionResult::SuccessServer
    }
}

impl BedBlock {
    pub fn set_occupied(
        occupied: bool,
        world: &Arc<World>,
        block: &Block,
        block_pos: &BlockPos,
        state_id: BlockStateId,
    ) -> bool {
        if state_id.to_block() != block {
            return false;
        }
        let mut bed_props = BedProperties::from_state_id(state_id);
        let other_half_pos = if bed_props.part == BedPart::Head {
            block_pos.offset(bed_props.facing.opposite().to_offset())
        } else {
            block_pos.offset(bed_props.facing.to_offset())
        };
        let other_state = world.get_block_state_id(&other_half_pos);
        if other_state.to_block() != block {
            return false;
        }
        let mut other_props = BedProperties::from_state_id(other_state);
        if other_props.part == bed_props.part || other_props.facing != bed_props.facing {
            return false;
        }
        bed_props.occupied = occupied;
        other_props.occupied = occupied;
        world.set_block_states_from_observed(&[
            (
                *block_pos,
                state_id,
                bed_props.to_state_id(block),
                BlockFlags::NOTIFY_LISTENERS,
            ),
            (
                other_half_pos,
                other_state,
                other_props.to_state_id(block),
                BlockFlags::NOTIFY_LISTENERS,
            ),
        ])
    }
}

fn entity_prevents_sleep(entity: &Entity) -> bool {
    NO_SLEEP_IDS.contains(&entity.entity_type.id)
}
