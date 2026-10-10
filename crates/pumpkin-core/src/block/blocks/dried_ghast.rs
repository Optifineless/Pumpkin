use std::sync::Arc;

use crate::{
    block::{
        BlockBehaviour, GetStateForNeighborUpdateArgs, OnPlaceArgs, OnScheduledTickArgs,
        PathComputationType, PlayerPlacedArgs, RandomTickArgs,
    },
    entity::{Entity, EntityBase, ageable::AgeableMob, passive::happy_ghast::HappyGhastEntity},
    world::World,
};
use pumpkin_data::{
    Block, BlockState, BlockStateId,
    block_properties::{DriedGhastLikeProperties, HorizontalFacing},
    entity::EntityType,
    fluid::Fluid,
    game_event::GameEvent,
    sound::{Sound, SoundCategory},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};

const MAX_HYDRATION_LEVEL: u8 = 3;
const HYDRATION_TICK_DELAY: u32 = 5000;

#[pumpkin_block("minecraft:dried_ghast")]
pub struct DriedGhastBlock;

impl DriedGhastBlock {
    /// Waterlogs a dried ghast for the water bucket container path.
    /// The state must belong to `DRIED_GHAST`; already waterlogged states return false.
    // DriedGhastBlock.placeLiquid.
    pub(crate) fn place_liquid(world: &Arc<World>, pos: &BlockPos, state: BlockStateId) -> bool {
        let mut props = DriedGhastLikeProperties::from_state_id(state);
        if props.waterlogged {
            return false;
        }
        props.waterlogged = true;
        world.set_block_state(
            pos,
            props.to_state_id(&Block::DRIED_GHAST),
            BlockFlags::NOTIFY_ALL,
        );
        world.schedule_fluid_tick(
            &Fluid::WATER,
            *pos,
            Fluid::WATER.flow_speed as u8,
            TickPriority::Normal,
        );
        world.play_sound(
            Sound::BlockDriedGhastPlaceInWater,
            SoundCategory::Blocks,
            &pos.to_f64(),
        );
        true
    }

    fn tick_waterlogged(world: &Arc<World>, pos: &BlockPos, mut props: DriedGhastLikeProperties) {
        // DriedGhastBlock.tickWaterlogged advances once per scheduled tick.
        if props.hydration == MAX_HYDRATION_LEVEL {
            Self::spawn_ghastling(world, pos, props.facing);
            return;
        }
        world.play_sound(
            Sound::BlockDriedGhastTransition,
            SoundCategory::Blocks,
            &pos.to_f64(),
        );
        props.hydration += 1;
        world.set_block_state(
            pos,
            props.to_state_id(&Block::DRIED_GHAST),
            BlockFlags::NOTIFY_LISTENERS,
        );
        world.emit_game_event(GameEvent::BlockChange.name(), pos.to_f64());
    }

    fn spawn_ghastling(world: &Arc<World>, pos: &BlockPos, facing: HorizontalFacing) {
        // DriedGhastBlock.spawnGhastling / Level.removeBlock preserve the water without loot.
        world.set_block_state(pos, Block::WATER.default_state.id, BlockFlags::NOTIFY_ALL);
        let spawn_at = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y),
            f64::from(pos.0.z) + 0.5,
        );
        let ghastling = HappyGhastEntity::new(Entity::new(
            world.clone(),
            spawn_at,
            &EntityType::HAPPY_GHAST,
        ));
        ghastling.set_baby(true);
        // Direction.getYRot.
        let yaw = match facing {
            HorizontalFacing::North => 180.0,
            HorizontalFacing::South => 0.0,
            HorizontalFacing::West => 90.0,
            HorizontalFacing::East => -90.0,
        };
        ghastling.get_entity().set_rotation(yaw, 0.0);
        if world.spawn_entity(ghastling) {
            world.play_sound(
                Sound::EntityGhastlingSpawn,
                SoundCategory::Blocks,
                &spawn_at,
            );
        }
    }
}

impl BlockBehaviour for DriedGhastBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        // DriedGhastBlock.getStateForPlacement uses the replaced source water and opposite yaw.
        let (fluid, state) = args.world.get_fluid_and_fluid_state(args.position);
        let mut props = DriedGhastLikeProperties::default(args.block);
        props.waterlogged = fluid.matches_type(&Fluid::WATER) && state.is_source;
        props.facing = args.player.get_entity().get_horizontal_facing().opposite();
        props.to_state_id(args.block)
    }

    fn player_placed(&self, args: PlayerPlacedArgs<'_>) {
        // DriedGhastBlock.setPlacedBy supplies the sound; SoundType.DRIED_GHAST's place is empty.
        let props = DriedGhastLikeProperties::from_state_id(args.state_id);
        let sound = if props.waterlogged {
            Sound::BlockDriedGhastPlaceInWater
        } else {
            Sound::BlockDriedGhastPlace
        };
        args.world
            .play_sound(sound, SoundCategory::Blocks, &args.position.to_f64());
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        // DriedGhastBlock.updateShape schedules contained water, keeping the block state.
        if DriedGhastLikeProperties::from_state_id(args.state_id).waterlogged {
            args.world.schedule_fluid_tick(
                &Fluid::WATER,
                *args.position,
                Fluid::WATER.flow_speed as u8,
                TickPriority::Normal,
            );
        }
        args.state_id
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        // DriedGhastBlock.randomTick also schedules partially hydrated dry blocks.
        let props =
            DriedGhastLikeProperties::from_state_id(args.world.get_block_state_id(args.position));
        if (props.waterlogged || props.hydration > 0)
            && !args
                .world
                .is_block_tick_scheduled(args.position, args.block)
        {
            args.world.level.schedule_block_tick_after(
                args.block,
                *args.position,
                HYDRATION_TICK_DELAY,
                TickPriority::Normal,
            );
        }
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        // DriedGhastBlock.tick dries by one level, without a transition sound.
        let mut props =
            DriedGhastLikeProperties::from_state_id(args.world.get_block_state_id(args.position));
        if props.waterlogged {
            Self::tick_waterlogged(args.world, args.position, props);
        } else if props.hydration > 0 {
            props.hydration -= 1;
            args.world.set_block_state(
                args.position,
                props.to_state_id(args.block),
                BlockFlags::NOTIFY_LISTENERS,
            );
            args.world
                .emit_game_event(GameEvent::BlockChange.name(), args.position.to_f64());
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
