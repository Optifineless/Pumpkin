use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::{
    block::fluid::{flowing_trait::FlowingFluid, water::FlowingWater},
    entity::{EntityBase, player::Player, r#type::from_type},
    item::{ItemBehaviour, ItemMetadata},
};
use pumpkin_data::{
    Block, BlockDirection,
    entity::EntityType,
    fluid::Fluid,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
};
use pumpkin_util::{
    GameMode, Hand,
    math::{position::BlockPos, vector3::Vector3},
};
use pumpkin_world::{tick::TickPriority, world::BlockFlags};
use uuid::Uuid;

use crate::world::World;

pub struct EmptyBucketItem;
pub struct FilledBucketItem;

impl ItemMetadata for EmptyBucketItem {
    fn ids() -> Box<[u16]> {
        [Item::BUCKET.id].into()
    }
}

impl ItemMetadata for FilledBucketItem {
    fn ids() -> Box<[u16]> {
        [
            Item::WATER_BUCKET.id,
            Item::LAVA_BUCKET.id,
            Item::POWDER_SNOW_BUCKET.id,
            Item::AXOLOTL_BUCKET.id,
            Item::COD_BUCKET.id,
            Item::SALMON_BUCKET.id,
            Item::TROPICAL_FISH_BUCKET.id,
            Item::PUFFERFISH_BUCKET.id,
            Item::TADPOLE_BUCKET.id,
        ]
        .into()
    }
}

fn get_start_and_end_pos(player: &Player, yaw: f32, pitch: f32) -> (Vector3<f64>, Vector3<f64>) {
    let start_pos = player.eye_position();
    let (yaw_rad, pitch_rad) = (f64::from(yaw.to_radians()), f64::from(pitch.to_radians()));
    let block_interaction_range = 4.5;
    let direction = Vector3::new(
        -yaw_rad.sin() * pitch_rad.cos() * block_interaction_range,
        -pitch_rad.sin() * block_interaction_range,
        pitch_rad.cos() * yaw_rad.cos() * block_interaction_range,
    );

    let end_pos = start_pos.add(&direction);
    (start_pos, end_pos)
}

const fn get_mob_for_bucket(item: &Item) -> Option<(&'static EntityType, Sound)> {
    if item.id == Item::AXOLOTL_BUCKET.id {
        Some((&EntityType::AXOLOTL, Sound::ItemBucketEmptyAxolotl))
    } else if item.id == Item::COD_BUCKET.id {
        Some((&EntityType::COD, Sound::ItemBucketEmptyFish))
    } else if item.id == Item::SALMON_BUCKET.id {
        Some((&EntityType::SALMON, Sound::ItemBucketEmptyFish))
    } else if item.id == Item::TROPICAL_FISH_BUCKET.id {
        Some((&EntityType::TROPICAL_FISH, Sound::ItemBucketEmptyFish))
    } else if item.id == Item::PUFFERFISH_BUCKET.id {
        Some((&EntityType::PUFFERFISH, Sound::ItemBucketEmptyFish))
    } else if item.id == Item::TADPOLE_BUCKET.id {
        Some((&EntityType::TADPOLE, Sound::ItemBucketEmptyTadpole))
    } else {
        None
    }
}

const fn get_empty_sound(item: &Item) -> Sound {
    if let Some((_, sound)) = get_mob_for_bucket(item) {
        sound
    } else if item.id == Item::LAVA_BUCKET.id {
        Sound::ItemBucketEmptyLava
    } else if item.id == Item::POWDER_SNOW_BUCKET.id {
        Sound::ItemBucketEmptyPowderSnow
    } else {
        Sound::ItemBucketEmpty
    }
}

// BucketItem/MobBucketItem.playEmptySound and SolidBucketItem.emptyContents.
fn play_empty_sound(world: &Arc<World>, player: Option<&Player>, item: &Item, pos: BlockPos) {
    let category = if get_mob_for_bucket(item).is_some() {
        SoundCategory::Neutral
    } else {
        SoundCategory::Blocks
    };
    if let Some(player) = player {
        world.play_block_sound_expect(player, get_empty_sound(item), category, pos);
    } else {
        world.play_block_sound(get_empty_sound(item), category, pos);
    }
    if get_mob_for_bucket(item).is_none() {
        world.emit_game_event(
            pumpkin_data::game_event::GameEvent::FluidPlace.name(),
            pos.to_f64(),
        );
    }
}

// MobBucketItem.checkExtraContent and spawn.
pub(crate) fn check_extra_content(world: &Arc<World>, stack: &ItemStack, pos: BlockPos) {
    if let Some((entity_type, _)) = get_mob_for_bucket(stack.item) {
        let spawn_coord = Vector3::new(
            f64::from(pos.0.x) + 0.5,
            f64::from(pos.0.y) + 1.0,
            f64::from(pos.0.z) + 0.5,
        );
        let entity = from_type(entity_type, spawn_coord, world, Uuid::new_v4());
        let spawn_coord =
            crate::entity::passive::bucketable::spawn_position(world, entity.as_ref(), pos);
        entity.get_entity().set_pos(spawn_coord);
        entity
            .get_entity()
            .set_rotation(rand::random::<f32>().mul_add(360.0, -180.0), 0.0);
        // MobBucketItem.spawn finalizes before Bucketable.loadFromBucketTag restores saved data.
        crate::entity::mob::spawn::finalize_spawn_with_reason(
            &entity,
            world,
            crate::entity::mob::spawn::SpawnReason::SpawnBucket,
            None,
        );
        if let Some(mob) = entity.get_mob() {
            // EntityType.create aligns the mob's body with its spawn yaw.
            mob.get_entity().body_yaw.store(mob.get_entity().yaw.load());
            crate::entity::passive::bucketable::load_from_bucket(mob, stack);
        }
        if !world.spawn_entity(entity.clone()) {
            return;
        }
        if let Some(mob) = entity.get_mob()
            && let Some(sound) = crate::entity::passive::bucketable::ambient_sound(mob)
        {
            // LivingEntity.makeSound/getVoicePitch.
            let base_pitch = if mob
                .as_ageable()
                .is_some_and(crate::entity::ageable::AgeableMob::is_baby)
            {
                1.5
            } else {
                1.0
            };
            let pitch = (rand::random::<f32>() - rand::random::<f32>()).mul_add(0.2, base_pitch);
            world.play_sound_raw(
                sound as u16,
                SoundCategory::Neutral,
                &spawn_coord,
                1.0,
                pitch,
            );
        }
        world.emit_game_event(
            pumpkin_data::game_event::GameEvent::EntityPlace.name(),
            pos.to_f64(),
        );
    }
}

const fn get_fill_sound(item: &Item) -> Sound {
    if item.id == Item::LAVA_BUCKET.id {
        Sound::ItemBucketFillLava
    } else if item.id == Item::POWDER_SNOW_BUCKET.id {
        Sound::ItemBucketFillPowderSnow
    } else {
        Sound::ItemBucketFill
    }
}

pub(crate) fn try_pickup_fluid_at(
    world: &Arc<World>,
    block_pos: BlockPos,
) -> Option<&'static Item> {
    let (block, state) = world.get_block_and_state_id(&block_pos);

    if block == &Block::POWDER_SNOW {
        world.break_block(
            &block_pos,
            None,
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
        );
        return Some(&Item::POWDER_SNOW_BUCKET);
    }

    if block.is_waterlogged(state) {
        let state_id = block.set_waterlogged(state, false).unwrap_or(state);
        world.set_block_state(&block_pos, state_id, BlockFlags::NOTIFY_ALL);
        world.schedule_fluid_tick(&Fluid::WATER, block_pos, 5, TickPriority::Normal);
        return Some(&Item::WATER_BUCKET);
    }

    if state == Block::LAVA.default_state.id || state == Block::WATER.default_state.id {
        world.break_block(&block_pos, None, BlockFlags::NOTIFY_ALL);
        world.set_block_state(
            &block_pos,
            Block::AIR.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        return Some(if state == Block::LAVA.default_state.id {
            &Item::LAVA_BUCKET
        } else {
            &Item::WATER_BUCKET
        });
    }

    None
}

fn bucket_pickup_target(
    world: &World,
    position: BlockPos,
    direction: BlockDirection,
) -> Option<(BlockPos, &'static Item)> {
    for pos in [position, position.offset(direction.to_offset())] {
        let (block, state) = world.get_block_and_state_id(&pos);
        let output = if block == &Block::POWDER_SNOW {
            Some(&Item::POWDER_SNOW_BUCKET)
        } else if block.is_waterlogged(state) || state == Block::WATER.default_state.id {
            Some(&Item::WATER_BUCKET)
        } else if state == Block::LAVA.default_state.id {
            Some(&Item::LAVA_BUCKET)
        } else {
            None
        };
        if let Some(output) = output {
            return Some((pos, output));
        }
    }
    None
}

pub(crate) const fn should_evaporate_in_nether(item: &Item, world: &World) -> bool {
    item.id != Item::LAVA_BUCKET.id
        && item.id != Item::POWDER_SNOW_BUCKET.id
        && world.dimension.water_evaporates
}

fn play_bucket_evaporation(world: &Arc<World>, player: Option<&Player>, pos: BlockPos) {
    let position = pos.to_centered_f64();
    let pitch = (rand::random::<f32>() - rand::random::<f32>()).mul_add(0.8, 2.6);
    if let Some(player) = player {
        world.play_sound_raw_expect(
            player,
            Sound::BlockFireExtinguish as u16,
            SoundCategory::Blocks,
            &position,
            0.5,
            pitch,
        );
    } else {
        world.play_sound_raw(
            Sound::BlockFireExtinguish as u16,
            SoundCategory::Blocks,
            &position,
            0.5,
            pitch,
        );
    }
}

// BucketItem.use/emptyContents dispatch on LiquidBlockContainer.
fn is_liquid_container(block: &Block) -> bool {
    block.is_waterloggable()
        || matches!(
            block.id,
            pumpkin_data::BlockId::KELP
                | pumpkin_data::BlockId::KELP_PLANT
                | pumpkin_data::BlockId::SEAGRASS
                | pumpkin_data::BlockId::TALL_SEAGRASS
        )
}

// BlockBehaviour.canBeReplaced(Fluid), LiquidBlockContainer.canPlaceLiquid.
pub(crate) fn can_empty_bucket_at(world: &World, item: &Item, pos: BlockPos) -> bool {
    let (block, state) = world.get_block_and_state(&pos);
    if item == &Item::POWDER_SNOW_BUCKET {
        return world.is_in_build_limit(pos) && state.is_air();
    }
    // EndPortalBlock and EndGatewayBlock override canBeReplaced(Fluid).
    if block == &Block::END_PORTAL || block == &Block::END_GATEWAY {
        return false;
    }
    state.is_air()
        || state.replaceable()
        || !state.is_solid()
        || (item != &Item::LAVA_BUCKET && block.is_waterloggable())
}

// BucketItem.use resolves the actual placement destination.
pub(crate) fn bucket_destination(
    world: &World,
    item: &Item,
    pos: BlockPos,
    direction: BlockDirection,
    sneaking: bool,
) -> Option<BlockPos> {
    // SolidBucketItem.useOn delegates to BlockItem, allowing replaceable player targets.
    if item == &Item::POWDER_SNOW_BUCKET {
        let destination = if world.get_block_state(&pos).replaceable() {
            pos
        } else {
            pos.offset(direction.to_offset())
        };
        let state = world.get_block_state(&destination);
        return (world.is_in_build_limit(destination)
            && (state.is_air() || state.is_liquid() || state.replaceable()))
        .then_some(destination);
    }
    // BucketItem.use tries the hit cell only for a compatible LiquidBlockContainer.
    let (block, _) = world.get_block_and_state(&pos);
    let inside = item != &Item::LAVA_BUCKET && is_liquid_container(block);
    let destination = if inside && !sneaking {
        pos
    } else {
        pos.offset(direction.to_offset())
    };
    can_empty_bucket_at(world, item, destination).then_some(destination)
}

pub(crate) fn empty_bucket_at(world: &Arc<World>, item: &Item, pos: BlockPos) -> Option<BlockPos> {
    empty_bucket_at_with_player(world, None, item, pos)
}

// BucketItem.emptyContents carries the nullable sound source through every placement branch.
fn empty_bucket_at_with_player(
    world: &Arc<World>,
    player: Option<&Player>,
    item: &Item,
    pos: BlockPos,
) -> Option<BlockPos> {
    if !can_empty_bucket_at(world, item, pos) {
        return None;
    }
    if should_evaporate_in_nether(item, world) {
        play_bucket_evaporation(world, player, pos);
        return Some(pos);
    }
    let (block, state) = world.get_block_and_state(&pos);
    if item != &Item::LAVA_BUCKET && item != &Item::POWDER_SNOW_BUCKET && is_liquid_container(block)
    {
        // SimpleWaterloggedBlock.placeLiquid can fail on an occupied container;
        // BucketItem.emptyContents deliberately ignores its result and still succeeds.
        if !block.is_waterlogged(state.id)
            && let Some(state_id) = block.set_waterlogged(state.id, true)
        {
            world.set_block_state(&pos, state_id, BlockFlags::NOTIFY_ALL);
            world.schedule_fluid_tick(
                &Fluid::WATER,
                pos,
                FlowingWater.get_flow_speed(world),
                TickPriority::Normal,
            );
        }
    } else {
        if !state.is_air() && !state.is_liquid() {
            world.break_block(&pos, None, BlockFlags::NOTIFY_ALL);
        }
        let state_id = if item == &Item::POWDER_SNOW_BUCKET {
            Block::POWDER_SNOW.default_state.id
        } else if item == &Item::LAVA_BUCKET {
            Block::LAVA.default_state.id
        } else {
            Block::WATER.default_state.id
        };
        world.set_block_state(&pos, state_id, BlockFlags::NOTIFY_ALL);
    }
    play_empty_sound(world, player, item, pos);
    Some(pos)
}

impl ItemBehaviour for EmptyBucketItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_rotation(item, player, yaw, pitch);
    }

    fn normal_use_with_rotation(&self, item: &Item, player: &Player, yaw: f32, pitch: f32) {
        self.normal_use_with_hand(item, player, yaw, pitch, Hand::Right);
    }

    // BucketItem.use reads and replaces the originating hand.
    fn normal_use_with_hand(
        &self,
        _block: &Item,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: Hand,
    ) {
        let world = player.world();
        let (start_pos, end_pos) = get_start_and_end_pos(player, yaw, pitch);

        let checker = |pos: &BlockPos, world_inner: &Arc<World>| {
            let state_id = world_inner.get_block_state_id(pos);

            let block = Block::from_state_id(state_id);

            if state_id == Block::AIR.default_state.id {
                return false;
            }

            (block.id != Block::WATER.id && block.id != Block::LAVA.id)
                || ((block.id == Block::WATER.id && state_id == Block::WATER.default_state.id)
                    || (block.id == Block::LAVA.id && state_id == Block::LAVA.default_state.id))
        };

        let Some((block_pos, direction)) = world.raycast(start_pos, end_pos, checker) else {
            return;
        };

        let Some((pickup_pos, item)) = bucket_pickup_target(&world, block_pos, direction) else {
            return;
        };

        if let Some(server) = world.server.upgrade()
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_bucket::PlayerBucketFillEvent::new(
                    player_arc,
                    block_pos,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }

        let mut stack = player.inventory().get_stack_in_hand(hand);
        if stack.is_empty()
            || stack.item != &Item::BUCKET
            || try_pickup_fluid_at(&world, pickup_pos).is_none()
        {
            return;
        }
        // BucketItem.use -> Player.playSound excludes the player and uses their sound source.
        world.play_sound_expect(
            player,
            get_fill_sound(item),
            SoundCategory::Players,
            &player.position(),
        );

        crate::item::item_utils::create_filled_result(
            &mut stack,
            player,
            ItemStack::new(1, item),
            true,
        );
        player.inventory().set_stack_in_hand(hand, stack);
        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(Item::BUCKET.id),
            1,
        );
    }

    fn use_on_entity(&self, item: &mut ItemStack, player: &Player, entity: Arc<dyn EntityBase>) {
        let ent = entity.get_entity();
        let entity_type = ent.entity_type;
        if !item.is_empty()
            && (entity_type == &EntityType::COW
                || entity_type == &EntityType::MOOSHROOM
                || entity_type == &EntityType::GOAT)
            && ent.age.load(Ordering::Relaxed) >= 0
        {
            let world = ent.world.load();
            let sound = if entity_type == &EntityType::GOAT {
                if let Some(goat) = entity
                    .cast_any()
                    .downcast_ref::<crate::entity::passive::goat::GoatEntity>()
                    && goat.is_screaming()
                {
                    Sound::EntityGoatScreamingMilk
                } else {
                    Sound::EntityGoatMilk
                }
            } else {
                Sound::EntityCowMilk
            };
            world.play_sound(sound, SoundCategory::Neutral, &ent.pos.load());
            crate::item::item_utils::create_filled_result(
                item,
                player,
                ItemStack::new(1, &Item::MILK_BUCKET),
                true,
            );
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// BucketItem.getEmptySuccessItem leaves creative stacks intact and replaces the requested hand.
fn finish_empty_bucket(player: &Player, hand: Hand, item: &Item) {
    if player.gamemode.load() != GameMode::Creative {
        let mut stack = player.inventory().get_stack_in_hand(hand);
        if stack.item != item {
            return;
        }
        crate::item::item_utils::create_filled_result(
            &mut stack,
            player,
            ItemStack::new(1, &Item::BUCKET),
            true,
        );
        player.inventory().set_stack_in_hand(hand, stack);
    }
    player.increment_stat(
        pumpkin_data::statistic::StatisticCategory::Used,
        i32::from(item.id),
        1,
    );
}

impl ItemBehaviour for FilledBucketItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        let (yaw, pitch) = player.rotation();
        self.normal_use_with_rotation(item, player, yaw, pitch);
    }

    fn normal_use_with_rotation(&self, item: &Item, player: &Player, yaw: f32, pitch: f32) {
        self.normal_use_with_hand(item, player, yaw, pitch, Hand::Right);
    }

    // BucketItem.use reads and replaces the originating hand.
    fn normal_use_with_hand(&self, item: &Item, player: &Player, yaw: f32, pitch: f32, hand: Hand) {
        let world = player.world();
        let (start_pos, end_pos) = get_start_and_end_pos(player, yaw, pitch);
        let checker = |pos: &BlockPos, world_inner: &Arc<World>| {
            let state_id = world_inner.get_block_state_id(pos);
            if Fluid::from_state_id(state_id).is_some() {
                return false;
            }
            state_id != Block::AIR.default_state.id
        };

        let Some((pos, direction)) = world.raycast(start_pos, end_pos, checker) else {
            return;
        };

        // BucketItem.use resolves eligibility, then plugin cancellation, before placement.
        let Some(destination) = bucket_destination(
            &world,
            item,
            pos,
            direction,
            player.get_entity().is_sneaking(),
        ) else {
            return;
        };
        if let Some(server) = world.server.upgrade()
            && let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_bucket::PlayerBucketEmptyEvent::new(
                    player_arc,
                    destination,
                    item.registry_key.to_string(),
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
        }

        let stack = player.inventory().get_stack_in_hand(hand);
        if stack.is_empty() || stack.item != item {
            return;
        }
        let place_pos = if item == &Item::POWDER_SNOW_BUCKET {
            world.set_block_state(
                &destination,
                Block::POWDER_SNOW.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            play_empty_sound(&world, Some(player), item, destination);
            destination
        } else if let Some(pos) =
            empty_bucket_at_with_player(&world, Some(player), item, destination)
        {
            pos
        } else {
            return;
        };
        check_extra_content(&world, &stack, place_pos);

        finish_empty_bucket(player, hand, item);
    }

    fn use_on_entity(&self, item: &mut ItemStack, player: &Player, entity: Arc<dyn EntityBase>) {
        if !item.is_empty()
            && item.item.id == Item::WATER_BUCKET.id
            && entity.get_entity().is_alive()
        {
            let entity_type = entity.get_entity().entity_type;
            let result_item = if entity_type == &EntityType::AXOLOTL {
                Some((&Item::AXOLOTL_BUCKET, Sound::ItemBucketFillAxolotl))
            } else if entity_type == &EntityType::COD {
                Some((&Item::COD_BUCKET, Sound::ItemBucketFillFish))
            } else if entity_type == &EntityType::SALMON {
                Some((&Item::SALMON_BUCKET, Sound::ItemBucketFillFish))
            } else if entity_type == &EntityType::TROPICAL_FISH {
                Some((&Item::TROPICAL_FISH_BUCKET, Sound::ItemBucketFillFish))
            } else if entity_type == &EntityType::PUFFERFISH {
                Some((&Item::PUFFERFISH_BUCKET, Sound::ItemBucketFillFish))
            } else if entity_type == &EntityType::TADPOLE {
                Some((&Item::TADPOLE_BUCKET, Sound::ItemBucketFillTadpole))
            } else {
                None
            };

            if let Some((mob_bucket, sound)) = result_item {
                let ent = entity.get_entity();
                let world = ent.world.load();
                world.play_sound(sound, SoundCategory::Neutral, &ent.pos.load());
                // Bucketable.bucketMobPickup has one owner of the transformed hand stack.
                crate::item::item_utils::create_filled_result(
                    item,
                    player,
                    ItemStack::new(1, mob_bucket),
                    false,
                );
                ent.remove();
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "bucket_sound_test_support.rs"]
mod sound_test_support;
#[cfg(test)]
#[path = "bucket_sound_tests.rs"]
mod sound_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        net::java::combat_test_support::TestPlayer, server::combat_test_support,
        world::spawn_test_support,
    };
    use pumpkin_data::{
        biome::Biome,
        data_component_impl::{BucketEntityDataImpl, CustomNameImpl},
    };
    use pumpkin_inventory::Inventory;
    use pumpkin_nbt::compound::NbtCompound;
    use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItem};
    use pumpkin_util::text::TextComponent;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[expect(
        clippy::unwrap_used,
        reason = "Regression fixture requires a released mob"
    )]
    async fn offhand_mob_bucket_use_restores_saved_data_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        chunk.set_block_state(8, 65, 11, Block::STONE.default_state);
        spawn_test_support::publish(&world, chunk);
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        player.inventory().set_stack(0, sword.clone());
        let mut tag = NbtCompound::new();
        tag.put_float("Health", 2.0);
        tag.put_bool("Silent", true);
        let mut bucket = ItemStack::new(1, &Item::COD_BUCKET);
        bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
        bucket.set_data_component(CustomNameImpl {
            name: TextComponent::text("London"),
        });
        player.inventory().set_stack(40, bucket);
        fixture.client().handle_use_item(
            player,
            &SUseItem {
                hand: VarInt(1),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &server,
        );
        assert_eq!(world.get_block(&BlockPos::new(8, 65, 10)), &Block::WATER);
        assert!(player.inventory().held_item().are_equal(&sword));
        assert_eq!(player.inventory().off_hand_item().item, &Item::BUCKET);
        let fish = world
            .entities
            .load()
            .iter()
            .find(|entity| entity.get_entity().entity_type == &EntityType::COD)
            .cloned()
            .unwrap();
        assert_eq!(fish.get_living_entity().unwrap().health.load(), 2.0);
        assert!(fish.get_entity().is_silent());
        assert_eq!(
            fish.get_entity()
                .custom_name
                .load()
                .as_ref()
                .as_ref()
                .map(|name| name.clone().get_text()),
            Some("London".to_owned())
        );
        assert!(fish.get_mob().unwrap().spawned_from_bucket());
        assert!(world.level.shutdown().await.is_ok());
    }
}
