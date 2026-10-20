use std::sync::Arc;

use pumpkin_data::{
    BlockId, BlockStateId,
    block_properties::{StickyPistonLikeProperties, WhiteBedLikeProperties},
    dimension::BedRule,
    entity::EntityType,
    statistic::{CustomStatistic, StatisticCategory},
    tag::{self, Taggable},
    translation,
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};

use super::BedBlock;
use crate::{
    block::{
        CanPlaceAtArgs, entities::shulker_box::ShulkerBoxBlockEntity, registry::BlockActionResult,
    },
    entity::{EntityBase, player::Player},
    plugin::api::events::player::player_bed::PlayerBedEnterEvent,
    world::World,
};

const BED_HORIZONTAL_REACH: f64 = 3.0;
const BED_VERTICAL_REACH: f64 = 2.0;
const MONSTER_HORIZONTAL_RANGE: f64 = 8.0;
const MONSTER_VERTICAL_RANGE: f64 = 5.0;

const NO_SLEEP_IDS: &[u16] = &[
    EntityType::BLAZE.id,
    EntityType::BOGGED.id,
    EntityType::SKELETON.id,
    EntityType::PARCHED.id,
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
    EntityType::PIGLIN.id,
    EntityType::PIGLIN_BRUTE.id,
    EntityType::PILLAGER.id,
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

impl BedBlock {
    pub(in crate::block::blocks) fn can_place_bed_at(args: &CanPlaceAtArgs<'_>) -> bool {
        let Some(player) = args.player else {
            return false;
        };
        let head = args
            .position
            .offset(player.get_entity().get_horizontal_facing().to_offset());
        let player_world = player.world();
        let world = args.world.unwrap_or(player_world.as_ref());
        args.block_accessor
            .get_block_state(args.position)
            .replaceable()
            && args.block_accessor.get_block_state(&head).replaceable()
            && world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_block(head.0.x, head.0.z)
    }

    // ServerPlayer.startSleepInBed: range and obstruction precede spawn, rules and monsters.
    pub(in crate::block::blocks) fn start_sleep_in_bed(
        world: &Arc<World>,
        player: &Arc<Player>,
        (head, foot, state): (BlockPos, BlockPos, BlockStateId),
        rule: BedRule,
        statistic: CustomStatistic,
    ) -> BlockActionResult {
        if WhiteBedLikeProperties::from_state_id(state).occupied {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_OCCUPIED,
                    translation::bedrock::TILE_BED_OCCUPIED
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }
        if !player.living_entity.is_alive() || player.sleeping_bed_pos.load().is_some() {
            return BlockActionResult::SuccessServer;
        }
        if !bed_in_range(player, head, foot) {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_TOO_FAR_AWAY,
                    translation::bedrock::TILE_BED_TOOFAR
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }
        if !free_at(world, head.up()) || !free_at(world, foot.up()) {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_OBSTRUCTED,
                    translation::bedrock::TILE_BED_OBSTRUCTED
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }
        if player.sleeping_bed_pos.load().is_some() {
            return BlockActionResult::SuccessServer;
        }
        let Some(admission) = player.bed_sleep_admission(world, head, state) else {
            return BlockActionResult::SuccessServer;
        };
        let is_dark = world.is_dark_outside();
        let can_set_spawn = rule.can_set_spawn(is_dark);
        if can_set_spawn {
            set_spawn(world, player, head);
        }
        if !player.bed_sleep_admission_is_current(&admission)
            || !bed_in_range(player, head, foot)
            || !free_at(world, head.up())
            || !free_at(world, foot.up())
        {
            return BlockActionResult::SuccessServer;
        }
        if !rule.can_sleep(world.is_dark_outside()) {
            if let Some(message) = rule.error_message
                && let Ok(message) = serde_json::from_str(message)
            {
                player.send_system_message_raw(&message, true);
            }
            return BlockActionResult::SuccessServer;
        }
        if !player.is_creative() && monsters_prevent_rest(world, player, head) {
            player.send_system_message_raw(
                &pumpkin_macros::translate_cross!(
                    translation::java::BLOCK_MINECRAFT_BED_NOT_SAFE,
                    translation::bedrock::TILE_BED_NOTSAFE
                ),
                true,
            );
            return BlockActionResult::SuccessServer;
        }
        if !player.bed_sleep_admission_is_current(&admission) {
            return BlockActionResult::SuccessServer;
        }
        if let Some(server) = world.server.upgrade() {
            let mut event = PlayerBedEnterEvent::new(player.clone(), head);
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return BlockActionResult::SuccessServer;
            }
        }
        if player
            .start_sleeping_in_bed(admission, || can_sleep_now(world, player, head, foot, rule))
        {
            player.increment_stat(StatisticCategory::Custom, statistic as i32, 1);
            if can_set_spawn {
                player.trigger_advancement(
                    crate::entity::player::advancement::trigger::AdvancementTrigger::SleptInBed,
                );
            }
        }
        BlockActionResult::SuccessServer
    }
}

// Plugin callbacks can move the actor, change the clock/headroom, or add a monster.
fn can_sleep_now(
    world: &World,
    player: &Player,
    head: BlockPos,
    foot: BlockPos,
    rule: BedRule,
) -> bool {
    player.sleeping_bed_pos.load().is_none()
        && bed_in_range(player, head, foot)
        && free_at(world, head.up())
        && free_at(world, foot.up())
        && rule.can_sleep(world.is_dark_outside())
        && (player.is_creative() || !monsters_prevent_rest(world, player, head))
}

fn bed_in_range(player: &Player, head: BlockPos, foot: BlockPos) -> bool {
    // BlockPos::to_f64 is Vec3.atBottomCenterOf, with inclusive bounds.
    let position = player.position();
    position.is_within_bounds(
        head.to_f64(),
        BED_HORIZONTAL_REACH,
        BED_VERTICAL_REACH,
        BED_HORIZONTAL_REACH,
    ) || position.is_within_bounds(
        foot.to_f64(),
        BED_HORIZONTAL_REACH,
        BED_VERTICAL_REACH,
        BED_HORIZONTAL_REACH,
    )
}

fn free_at(world: &World, position: BlockPos) -> bool {
    !is_suffocating(world, position)
}

fn is_suffocating(world: &World, position: BlockPos) -> bool {
    let (block, state) = world.get_block_and_state(&position);
    // Blocks' explicit Properties.isSuffocating overrides precede the default predicate.
    if matches!(
        block.id,
        BlockId::FARMLAND | BlockId::SOUL_SAND | BlockId::DIRT_PATH | BlockId::MUD
    ) {
        return true;
    }
    if block.has_tag(&tag::Block::MINECRAFT_LEAVES)
        || block.has_tag(&tag::Block::C_GLASS_BLOCKS)
        || matches!(
            block.id,
            BlockId::MANGROVE_ROOTS
                | BlockId::MOVING_PISTON
                | BlockId::COPPER_GRATE
                | BlockId::EXPOSED_COPPER_GRATE
                | BlockId::WEATHERED_COPPER_GRATE
                | BlockId::OXIDIZED_COPPER_GRATE
                | BlockId::WAXED_COPPER_GRATE
                | BlockId::WAXED_EXPOSED_COPPER_GRATE
                | BlockId::WAXED_WEATHERED_COPPER_GRATE
                | BlockId::WAXED_OXIDIZED_COPPER_GRATE
        )
    {
        return false;
    }
    if matches!(block.id, BlockId::PISTON | BlockId::STICKY_PISTON) {
        return !StickyPistonLikeProperties::from_state_id(state.id).extended;
    }
    if block.has_tag(&tag::Block::MINECRAFT_SHULKER_BOXES) {
        // Pumpkin currently represents open shulkers by their viewer count, without lid progress.
        return world
            .get_block_entity(&position)
            .and_then(|entity| {
                entity
                    .as_any()
                    .downcast_ref::<ShulkerBoxBlockEntity>()
                    .map(|shulker| {
                        shulker
                            .viewers
                            .current
                            .load(std::sync::atomic::Ordering::Relaxed)
                            == 0
                    })
            })
            .unwrap_or(true);
    }
    // BlockBehaviour.Properties' default is tag membership and a full collision shape.
    block.has_tag(&tag::Block::MINECRAFT_CAUSES_SUFFOCATION) && state.is_full_cube()
}

fn set_spawn(world: &Arc<World>, player: &Player, head: BlockPos) {
    if player.set_respawn_point(
        world.dimension.clone(),
        head,
        player.get_entity().yaw.load(),
        player.get_entity().pitch.load(),
        false,
    ) {
        player.send_system_message(&pumpkin_macros::translate_cross!(
            translation::java::BLOCK_MINECRAFT_SET_SPAWN,
            translation::bedrock::TILE_BED_RESPAWNSET
        ));
    }
}

fn monsters_prevent_rest(world: &World, player: &Player, head: BlockPos) -> bool {
    let center = head.to_f64();
    let range = BoundingBox::new(
        center.add_raw(
            -MONSTER_HORIZONTAL_RANGE,
            -MONSTER_VERTICAL_RANGE,
            -MONSTER_HORIZONTAL_RANGE,
        ),
        center.add_raw(
            MONSTER_HORIZONTAL_RANGE,
            MONSTER_VERTICAL_RANGE,
            MONSTER_HORIZONTAL_RANGE,
        ),
    );
    world
        .get_entities_at_box(&range)
        .iter()
        .any(|entity| entity_prevents_sleep(entity.as_ref(), player, world))
}

fn entity_prevents_sleep(entity: &dyn EntityBase, player: &Player, world: &World) -> bool {
    let entity_type = entity.get_entity().entity_type;
    if entity_type == &EntityType::ZOMBIFIED_PIGLIN {
        // ZombifiedPiglin.isPreventingPlayerRest overrides Monster's unconditional predicate.
        return entity
            .get_mob()
            .and_then(|mob| mob.as_neutral())
            .is_some_and(|neutral| neutral.is_angry_at(player, world));
    }
    NO_SLEEP_IDS.contains(&entity_type.id)
}
