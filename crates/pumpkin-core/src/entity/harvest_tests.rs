// shared fixtures for the harvest regression tests, without starting a server.
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering::Relaxed},
};

use pumpkin_data::{Block, BlockDirection, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};

use super::{Entity, EntityBase, death_test_world::DeathTestWorld};
use crate::{
    block::entities::{BlockEntity, brewing_stand::BrewingStandBlockEntity},
    plugin::{BoxFuture, EventHandler, EventPriority, Payload},
    server::Server,
};

struct Handler<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Handler<F> {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut E,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
fn register<E: Payload + Send + Sync + 'static>(
    server: &Server,
    action: impl Fn(&mut E) + Send + Sync + 'static,
) {
    server
        .plugin_manager
        .register::<E, _>(Arc::new(Handler(action)), EventPriority::Normal, true);
}
fn add_chunk(world: &crate::world::World) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), Arc::new(ChunkData::empty(0, 0)));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression fixture requires real bucketable mobs"
)]
async fn released_bucket_keeps_name_health_age_variant_and_persistence() {
    use pumpkin_data::data_component_impl::{
        AxolotlVariantImpl, BucketEntityDataImpl, CustomNameImpl,
    };
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut tag = NbtCompound::new();
    tag.put_float("Health", 7.0);
    tag.put_int("Age", -1234);
    tag.put_bool("AgeLocked", true);
    let mut bucket = ItemStack::new(1, &Item::AXOLOTL_BUCKET);
    bucket.set_data_component(BucketEntityDataImpl { nbt: Some(tag) });
    bucket.set_data_component(CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("London"),
    });
    bucket.set_data_component(AxolotlVariantImpl {
        value: "blue".into(),
    });
    crate::item::items::bucket::check_extra_content(&world, &bucket, BlockPos::new(4, 64, 4));
    let mob = world
        .entities
        .load()
        .iter()
        .find(|entity| entity.get_entity().entity_type == &EntityType::AXOLOTL)
        .cloned()
        .unwrap();
    assert_eq!(
        mob.get_entity()
            .custom_name
            .load()
            .as_ref()
            .as_ref()
            .map(|name| name.clone().get_text()),
        Some("London".to_owned())
    );
    assert_eq!(mob.get_living_entity().unwrap().health.load(), 7.0);
    assert_eq!(mob.get_entity().age.load(Relaxed), -1234);
    let axolotl = mob
        .cast_any()
        .downcast_ref::<super::passive::axolotl::AxolotlEntity>()
        .unwrap();
    assert_eq!(
        axolotl.get_variant(),
        super::passive::axolotl::AxolotlVariant::Blue
    );
    assert!(axolotl.is_from_bucket());
    let mut saved = NbtCompound::new();
    mob.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_bool("FromBucket"), Some(true));
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bucket_replaces_plants_and_returns_the_waterlogged_destination() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let pos = BlockPos::new(4, 64, 4);
    world.set_block_state(&pos, Block::KELP.default_state.id, BlockFlags::FORCE_STATE);
    assert_eq!(
        crate::item::items::bucket::bucket_destination(
            &world,
            &Item::WATER_BUCKET,
            pos,
            BlockDirection::North,
            false
        ),
        Some(pos)
    );
    assert_eq!(
        crate::item::items::bucket::empty_bucket_at(&world, &Item::WATER_BUCKET, pos),
        Some(pos)
    );
    assert_eq!(world.get_block(&pos), &Block::KELP);
    world.set_block_state(
        &pos,
        Block::SHORT_GRASS.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_eq!(
        crate::item::items::bucket::empty_bucket_at(&world, &Item::WATER_BUCKET, pos),
        Some(pos)
    );
    assert_eq!(world.get_block(&pos), &Block::WATER);
    world.set_block_state(
        &pos,
        Block::OAK_SLAB.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    assert_eq!(
        crate::item::items::bucket::bucket_destination(
            &world,
            &Item::WATER_BUCKET,
            pos,
            BlockDirection::Up,
            false
        ),
        Some(pos)
    );
    assert_eq!(
        crate::item::items::bucket::empty_bucket_at(&world, &Item::WATER_BUCKET, pos),
        Some(pos)
    );
    assert!(Block::OAK_SLAB.is_waterlogged(world.get_block_state_id(&pos)));
    assert_eq!(
        crate::item::items::bucket::empty_bucket_at(&world, &Item::WATER_BUCKET, pos),
        Some(pos)
    );
    let bucket = ItemStack::new(1, &Item::AXOLOTL_BUCKET);
    crate::item::items::bucket::check_extra_content(&world, &bucket, pos);
    let heights: Vec<_> = world
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::AXOLOTL)
        .map(|entity| entity.get_entity().pos.load().y)
        .collect();
    assert_eq!(heights, vec![64.5]);
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_falling_block_placement_removes_it_without_a_drop() {
    use crate::plugin::entity::entity_change_block::EntityChangeBlockEvent;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let support = BlockPos::new(4, 63, 4);
    world.set_block_state(
        &support,
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    register::<EntityChangeBlockEvent>(&fixture.server, |event| {
        assert_eq!(event.new_block, "sand");
        event.cancelled = true;
    });
    let sand = super::falling::FallingEntity::new(
        Entity::new(
            world.clone(),
            Vector3::new(4.5, 64.0, 4.5),
            &EntityType::FALLING_BLOCK,
        ),
        Block::SAND.default_state.id,
    );
    sand.tick(&sand, &fixture.server);
    assert!(!sand.get_entity().is_alive());
    assert_eq!(world.get_block(&support.up()), &Block::AIR);
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_entity().entity_type != &EntityType::ITEM)
    );
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn brewing_completion_waits_for_the_next_start_event() {
    use crate::plugin::block::brewing_start::BrewingStartEvent;
    use pumpkin_protocol::codec::recipe::{DynamicRecipe, OwnedBrewingRecipe};
    let fixture = DeathTestWorld::new().await;
    fixture
        .server
        .recipe_manager
        .add_recipe(DynamicRecipe::Brewing(OwnedBrewingRecipe {
            recipe_id: "test:repeat".into(),
            input_item: "minecraft:potion".into(),
            input_potion: None,
            reagent: "minecraft:cobblestone".into(),
            output_item: "minecraft:potion".into(),
            output_potion: None,
        }));
    let world = fixture.world();
    add_chunk(&world);
    let pos = BlockPos::new(4, 64, 4);
    world.set_block_state(
        &pos,
        Block::BREWING_STAND.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let stand = BrewingStandBlockEntity::new(pos);
    stand.set_stack(0, ItemStack::new(1, &Item::POTION));
    stand.set_stack(3, ItemStack::new(3, &Item::COBBLESTONE));
    stand.fuel.store(3, Relaxed);
    let starts = Arc::new(AtomicU32::new(0));
    let count = starts.clone();
    register::<BrewingStartEvent>(&fixture.server, move |event| {
        count.fetch_add(1, Relaxed);
        event.brewing_time = 1;
        if count.load(Relaxed) == 2 {
            event.cancelled = true;
        }
    });
    stand.tick(&world);
    stand.tick(&world);
    assert_eq!(stand.brew_time.load(Relaxed), 0);
    assert_eq!(stand.fuel.load(Relaxed), 2);
    stand.tick(&world);
    assert_eq!(starts.load(Relaxed), 2);
    assert_eq!(stand.fuel.load(Relaxed), 2);
    assert!(stand.is_valid_slot_for(3, &ItemStack::new(1, &Item::BLAZE_POWDER)));
    assert!(!stand.is_valid_slot_for(3, &ItemStack::new(1, &Item::DIAMOND_PICKAXE)));
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(clippy::unwrap_used, reason = "Recorded real teleport packet bytes")]
async fn teleport_cancellation_and_modified_destination_control_the_chunk_view() {
    use crate::plugin::player::player_teleport::PlayerTeleportEvent;
    use pumpkin_protocol::ser::NetworkReadExt;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mut witness = crate::net::java::combat_test_support::TestPlayer::new(&world);
    let player = fixture.player("teleporter");
    witness.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            Vector2::new(10, 10),
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
    witness.take_packets();
    let destination = Vector3::new(160.0, 64.0, 160.0);
    register::<PlayerTeleportEvent>(&fixture.server, move |event| {
        event.to = destination;
        event.cancelled = event.player.chunk_send_epoch.load(Relaxed) > 0;
    });
    player.teleport(
        Vector3::new(32.0, 64.0, 32.0),
        Some(0.0),
        Some(0.0),
        world.clone(),
    );
    let mut broadcasts = Vec::new();
    for bytes in witness.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::ENTITY_POSITION_SYNC.0
        {
            assert_eq!(data.get_var_int().unwrap().0, player.entity_id());
            assert_eq!(data.get_var_int().unwrap().0, 0); // linear position path
            broadcasts.push(Vector3::new(
                data.get_f64_be().unwrap(),
                data.get_f64_be().unwrap(),
                data.get_f64_be().unwrap(),
            ));
        }
    }
    assert_eq!(broadcasts, vec![destination]);
    assert_eq!(player.watched_section.load().center, Vector2::new(10, 10));
    assert_eq!(
        player.request_teleport(Vector3::new(320.0, 64.0, 320.0), 0.0, 0.0),
        None
    );
    witness.take_packets();
    player.teleport(
        Vector3::new(320.0, 64.0, 320.0),
        Some(0.0),
        Some(0.0),
        world,
    );
    assert!(witness.take_packets().is_empty());
    assert_eq!(player.position(), destination);
    assert_eq!(player.chunk_send_epoch.load(Relaxed), 1);
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression fixture requires a wither and item"
)]
async fn wither_death_drops_one_extended_lifetime_star_at_its_position() {
    let fixture = DeathTestWorld::new().await;
    let wither = fixture.mob(&EntityType::WITHER);
    let position = Vector3::new(2.25, 65.75, 3.125);
    wither.get_entity().set_pos(position);
    let living = wither.get_living_entity().unwrap();
    living.set_health(0.0);
    living.on_death(pumpkin_data::damage::DamageType::GENERIC, None, None);
    living.on_death(pumpkin_data::damage::DamageType::GENERIC, None, None);
    wither.get_mob().unwrap().post_tick();
    let world = fixture.world();
    let entities = world.entities.load();
    let stars: Vec<_> = entities
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<super::item::ItemEntity>())
        .filter(|entity| entity.get_item_stack().lock().unwrap().item == &Item::NETHER_STAR)
        .collect();
    assert_eq!(stars.len(), 1);
    assert_eq!(stars[0].get_entity().pos.load(), position);
    let mut saved = NbtCompound::new();
    stars[0].write_custom_nbt(&mut saved);
    assert_eq!(saved.get_short("Age"), Some(-6000));
    crate::server::fixture_lifecycle::finish().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_bucket_emptying_keeps_the_block_and_full_bucket() {
    use crate::{
        item::{ItemBehaviour, items::bucket::FilledBucketItem},
        plugin::player::player_bucket::PlayerBucketEmptyEvent,
    };
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    add_chunk(&world);
    let player = fixture.player("bucket-user");
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 2.5));
    player.get_entity().set_rotation(0.0, 0.0);
    player
        .inventory
        .set_slot(0, ItemStack::new(1, &Item::WATER_BUCKET));
    world.set_block_state(
        &BlockPos::new(4, 65, 4),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let calls = Arc::new(AtomicU32::new(0));
    let seen = calls.clone();
    register::<PlayerBucketEmptyEvent>(&fixture.server, move |event| {
        seen.fetch_add(1, Relaxed);
        assert_eq!(event.block_pos, BlockPos::new(4, 65, 3));
        event.cancelled = true;
    });
    FilledBucketItem.normal_use(&Item::WATER_BUCKET, &player);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(world.get_block(&BlockPos::new(4, 65, 3)), &Block::AIR);
    assert_eq!(player.inventory.held_item().item, &Item::WATER_BUCKET);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(clippy::unwrap_used, reason = "Regression fixture requires a wither")]
async fn wither_mob_tick_respects_disabled_mob_drops() {
    let fixture = DeathTestWorld::new().await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.mob_drops = false;
        info
    });
    let wither = fixture.mob(&EntityType::WITHER);
    let living = wither.get_living_entity().unwrap();
    living.set_health(0.0);
    living.on_death(pumpkin_data::damage::DamageType::GENERIC, None, None);
    wither.get_mob().unwrap().mob_tick(wither.as_ref());
    assert!(fixture.world().entities.load().iter().all(|entity| {
        entity
            .cast_any()
            .downcast_ref::<super::item::ItemEntity>()
            .is_none_or(|item| item.get_item_stack().lock().unwrap().item != &Item::NETHER_STAR)
    }));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression fixture requires a brewing menu"
)]
async fn occupied_brewing_slots_allow_swaps_and_keep_failed_quick_moves_in_place() {
    use pumpkin_inventory::{
        brewing::brewing_screen_handler::BrewingScreenHandler, screen_handler::ScreenHandler,
    };
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("brewer");
    let stand = Arc::new(BrewingStandBlockEntity::new(BlockPos::new(4, 64, 4)));
    for slot in 0..3 {
        stand.set_stack(slot, ItemStack::new(1, &Item::POTION));
    }
    player
        .inventory
        .set_slot(9, ItemStack::new(1, &Item::SPLASH_POTION));
    let inventory: Arc<dyn Inventory> = stand.clone();
    let properties = stand.to_property_delegate().unwrap();
    let mut menu = BrewingScreenHandler::new(1, &player.inventory, inventory.clone(), &properties);
    assert!(menu.get_behaviour().slots[0].can_insert(&ItemStack::new(1, &Item::SPLASH_POTION)));
    assert!(!inventory.is_valid_slot_for(0, &ItemStack::new(1, &Item::SPLASH_POTION)));
    assert!(menu.quick_move(player.as_ref(), 5).is_empty());
    assert_eq!(player.inventory.get_stack(9).item, &Item::SPLASH_POTION);
    assert!(player.inventory.get_stack(0).is_empty());
    crate::server::fixture_lifecycle::finish().await;
}
