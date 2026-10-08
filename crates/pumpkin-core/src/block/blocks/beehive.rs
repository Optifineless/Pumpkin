use crate::entity::ageable::AgeableMob;
use crate::{
    block::{
        BlockBehaviour, BlockMetadata, GetComparatorOutputArgs, UseWithItemArgs,
        entities::beehive::BeehiveBlockEntity, registry::BlockActionResult,
    },
    entity::r#type::from_type,
    world::World,
};
use pumpkin_data::HorizontalFacingExt;
use pumpkin_data::{
    Block, BlockId,
    block_properties::{BeeNestLikeProperties, CampfireLikeProperties},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    tag::Taggable,
};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};
use pumpkin_world::world::BlockFlags;
use rand::RngExt;
use std::sync::Arc;

pub struct BeehiveBlock;

impl BlockMetadata for BeehiveBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::BEEHIVE, BlockId::BEE_NEST].into()
    }
}

impl BlockBehaviour for BeehiveBlock {
    // BeehiveBlock.useItemOn bottles honey, then applies smoke or emergency bee release.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state = args.world.get_block_state_id(args.position);
        let props = BeeNestLikeProperties::from_state_id(state);
        if props.honey_level < 5 || args.item_stack.item != &Item::GLASS_BOTTLE {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let mut event =
            crate::plugin::api::events::player::player_harvest_block::PlayerHarvestBlockEvent {
                player: args.player.clone(),
                block_pos: *args.position,
                harvested_items: vec![ItemStack::new(1, &Item::HONEY_BOTTLE)],
                cancelled: false,
            };
        if let Some(server) = args.world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled {
            return BlockActionResult::Fail;
        }
        args.player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(args.item_stack.item.id),
            1,
        );
        // Vanilla shrinks bottles even in creative; this interaction is not ItemUtils.createFilledResult.
        args.item_stack.decrement(1);
        for output in event.harvested_items {
            if args.item_stack.is_empty() {
                *args.item_stack = output;
            } else {
                crate::item::item_utils::give_or_drop(args.player, output);
            }
        }
        args.world.play_sound(
            Sound::ItemBottleFill,
            SoundCategory::Blocks,
            &args.player.position(),
        );
        args.world
            .emit_game_event("fluid_pickup", args.position.to_centered_f64());
        finish_harvest(args.world, *args.position, args.player);
        BlockActionResult::Success
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        Some(
            BeeNestLikeProperties::from_state_id(args.world.get_block_state_id(args.position))
                .honey_level,
        )
    }
}

/// Checks campfire smoke using CampfireBlock.isSmokeyPos's central collision column.
pub fn is_smokey_pos(world: &World, position: BlockPos) -> bool {
    // CampfireBlock.SHAPE_VIRTUAL_POST is Block.column(4, 0, 16).
    let smoke_column = BoundingBox::new_array([0.375, 0.0, 0.375], [0.625, 1.0, 0.625]);
    for distance in 1..=5 {
        let below = position.down_height(distance);
        let state = world.get_block_state(&below);
        if is_lit_campfire(state.id) {
            return true;
        }
        if state
            .get_block_collision_shapes()
            .any(|shape| smoke_column.intersects(&shape))
        {
            return is_lit_campfire(world.get_block_state_id(&below.down()));
        }
    }
    false
}

fn is_lit_campfire(state: pumpkin_data::BlockStateId) -> bool {
    Block::from_state_id(state).has_tag(&pumpkin_data::tag::Block::MINECRAFT_CAMPFIRES)
        && CampfireLikeProperties::from_state_id(state).lit
}

/// Resets honey and releases hive occupants when the harvest is not protected by smoke.
/// Mirrors BeehiveBlock.releaseBeesAndResetHoneyLevel and BeehiveBlockEntity.emptyAllLivingFromHive.
pub fn finish_harvest(
    world: &Arc<World>,
    position: BlockPos,
    player: &Arc<crate::entity::player::Player>,
) {
    let (block, state) = world.get_block_and_state_id(&position);
    let mut props = BeeNestLikeProperties::from_state_id(state);
    props.honey_level = 0;
    world.set_block_state(&position, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
    if is_smokey_pos(world, position) {
        return;
    }
    let Some(entity) = world.get_block_entity(&position) else {
        return;
    };
    let Some(hive) = entity.as_any().downcast_ref::<BeehiveBlockEntity>() else {
        return;
    };
    let occupants = hive
        .bees
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .unwrap_or_default();
    if !occupants.is_empty() {
        anger_nearby_bees(world, position);
    }
    let mut remaining = Vec::new();
    for occupant in occupants {
        if !release_occupant(world, position, props, player, &occupant) {
            remaining.push(occupant);
        }
    }
    restore_failed_occupants(hive, remaining);
    world.update_block_entity(&entity);
}

// BeehiveBlockEntity.emptyAllLivingFromHive retains occupants that cannot be released.
fn restore_failed_occupants(hive: &BeehiveBlockEntity, remaining: Vec<pumpkin_nbt::tag::NbtTag>) {
    hive.bees
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_default()
        .extend(remaining);
}

fn release_occupant(
    world: &Arc<World>,
    position: BlockPos,
    props: BeeNestLikeProperties,
    player: &Arc<crate::entity::player::Player>,
    occupant: &pumpkin_nbt::tag::NbtTag,
) -> bool {
    let Some(occupant) = occupant.extract_compound() else {
        return false;
    };
    let Some(data) = occupant
        .get_compound("entity_data")
        .or_else(|| occupant.get_compound("EntityData"))
    else {
        return false;
    };
    let Some(entity_type) = data.get_string("id").and_then(EntityType::from_name) else {
        return false;
    };
    if !entity_type.has_tag(&pumpkin_data::tag::EntityType::MINECRAFT_BEEHIVE_INHABITORS) {
        return false;
    }
    let entity = from_type(
        entity_type,
        position.to_centered_f64(),
        world,
        uuid::Uuid::new_v4(),
    );
    restore_occupant(
        &entity,
        data,
        occupant.get_int("ticks_in_hive").unwrap_or(0),
    );
    let facing = props.facing.to_block_direction();
    let blocked = world
        .get_block_state(&position.offset(facing.to_offset()))
        .get_block_collision_shapes()
        .next()
        .is_some();
    let size = entity.get_entity().entity_dimension.load();
    let delta = if blocked {
        0.0
    } else {
        0.55 + f64::from(size.width) / 2.0
    };
    let offset = facing.to_offset();
    entity
        .get_entity()
        .set_pos(position.to_centered_f64().add_raw(
            delta * f64::from(offset.x),
            -f64::from(size.height) / 2.0,
            delta * f64::from(offset.z),
        ));
    if let Some(mob) = entity.get_mob()
        && player
            .position()
            .squared_distance_to_vec(&entity.get_entity().pos.load())
            <= 16.0
    {
        mob.get_mob_entity().set_target(Some(player.clone()));
    }
    world.play_sound(
        Sound::BlockBeehiveExit,
        SoundCategory::Blocks,
        &position.to_centered_f64(),
    );
    world.emit_game_event("block_change", position.to_centered_f64());
    world.spawn_entity(entity);
    true
}

fn anger_nearby_bees(world: &World, position: BlockPos) {
    // BeehiveBlock.angerNearbyBees searches eight blocks horizontally and six vertically.
    let area = BoundingBox::from_block(&position).expand(8.0, 6.0, 8.0);
    let entities = world.get_entities_at_box(&area);
    let players = world.get_players_at_box(&area);
    if players.is_empty() {
        return;
    }
    for entity in entities {
        if entity.get_entity().entity_type == &EntityType::BEE
            && let Some(mob) = entity.get_mob()
            && mob.get_mob_entity().get_target().is_none()
        {
            mob.get_mob_entity().set_target(Some(
                players[rand::rng().random_range(0..players.len())].clone(),
            ));
        }
    }
}

// BeehiveBlockEntity.Occupant.createEntity strips IGNORED_BEE_TAGS and updates age on release.
fn restore_occupant(
    entity: &Arc<dyn crate::entity::EntityBase>,
    data: &pumpkin_nbt::compound::NbtCompound,
    ticks_in_hive: i32,
) {
    const IGNORED_BEE_TAGS: &[&str] = &[
        "Air",
        "drop_chances",
        "equipment",
        "Brain",
        "CanPickUpLoot",
        "DeathTime",
        "fall_distance",
        "FallFlying",
        "Fire",
        "HurtTime",
        "LeftHanded",
        "Motion",
        "NoGravity",
        "OnGround",
        "PortalCooldown",
        "Pos",
        "Rotation",
        "sleeping_pos",
        "CannotEnterHiveTicks",
        "TicksSincePollination",
        "CropsGrownSincePollination",
        "hive_pos",
        "Passengers",
        "leash",
        "UUID",
    ];
    let mut data = data.clone();
    for name in IGNORED_BEE_TAGS {
        data.child_tags.remove(*name);
    }
    entity.read_nbt_non_mut(&data);
    entity.get_entity().set_has_no_gravity(true);
    if let Some(bee) = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::bee::BeeEntity>()
    {
        bee.mob_entity
            .love_ticks
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |ticks| Some(ticks.saturating_sub(ticks_in_hive).max(0)),
            )
            .ok();
        if !bee.is_age_locked() {
            let age = bee.get_age();
            bee.set_age(if age < 0 {
                age.saturating_add(ticks_in_hive).min(0)
            } else {
                age.saturating_sub(ticks_in_hive).max(0)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_nbt::tag::NbtTag;

    #[test]
    fn review_hive_keeps_bees_entering_during_release() {
        let hive = BeehiveBlockEntity::new(BlockPos::new(0, 64, 0));
        let incoming = NbtTag::String("incoming occupant".into());
        let failed = NbtTag::String("failed occupant".into());
        *hive.bees.lock().unwrap() = Some(vec![incoming.clone()]);
        restore_failed_occupants(&hive, vec![failed.clone()]);
        assert_eq!(*hive.bees.lock().unwrap(), Some(vec![incoming, failed]));
    }
}
