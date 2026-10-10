use super::*;
use crate::{
    entity::{Entity, RemovalReason},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::vehicle::vehicle_destroy::VehicleDestroyEvent,
    },
    world::portal::{PortalProcessor, PortalType},
};
use pumpkin_data::{BlockDirection, damage::DamageType};
use pumpkin_util::Hand;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_boat_portal_keeps_rider_vehicle_and_contents_in_source_world() {
    let fixture = Fixture::new();
    let rider = TestPlayer::new(&fixture.world);
    fixture
        .world
        .players
        .store(Arc::new(vec![rider.player.clone()]));
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    rider
        .player
        .get_entity()
        .set_pos(boat.get_entity().pos.load());
    assert!(boat.interact(&rider.player, &mut ItemStack::EMPTY.clone()));
    let pos = boat.get_entity().pos.load();
    let portal = BlockPos::new(8, 64, 8);
    fixture.world.set_block_state(
        &portal,
        Block::NETHER_PORTAL.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let destination_dir = tempfile::tempdir().unwrap();
    let destination = combat_test_support::world(&fixture.server, destination_dir.path());
    *boat.get_entity().portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::Nether,
        portal,
        destination.clone(),
    ));
    boat.tick(boat.as_ref(), &fixture.server);
    assert!(boat.get_entity().portal_manager.lock().unwrap().is_none());
    assert!(Arc::ptr_eq(
        &boat.get_entity().world.load_full(),
        &fixture.world
    ));
    assert!(Arc::ptr_eq(&rider.player.world(), &fixture.world));
    assert_eq!(boat.get_entity().pos.load(), pos);
    assert_eq!(rider.player.get_entity().pos.load(), pos);
    assert!(rider.player.get_entity().has_vehicle());
    let mut nbt = NbtCompound::new();
    boat.write_custom_nbt(&mut nbt);
    let NbtTag::Compound(stack) = &nbt.get_list("Items").unwrap()[0] else {
        panic!("chest boat saved a non-compound item")
    };
    assert_eq!(stack.get_int("count"), Some(3));
    assert_eq!(stack.get_string("id").unwrap(), "minecraft:diamond");
    assert_eq!(fixture.drops(&Item::DIAMOND), 0);
    destination.level.shutdown().await.unwrap();
    fixture.shutdown().await;
}

struct UnloadDuringDestroy(Arc<dyn EntityBase>);
impl EventHandler<VehicleDestroyEvent> for UnloadDuringDestroy {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleDestroyEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {
            let world = self.0.get_entity().world.load_full();
            world
                .remove_entities_in_chunks([self.0.get_entity().chunk_pos.load()])
                .await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_winning_during_destroy_event_drops_no_vehicle_or_contents() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    fixture.server.plugin_manager.register(
        Arc::new(UnloadDuringDestroy(boat.clone())),
        EventPriority::Normal,
        true,
    );
    assert!(boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK));
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 0);
    assert_eq!(fixture.drops(&Item::DIAMOND), 0);
    assert!(boat.get_entity().removal_reason.load() == Some(RemovalReason::UnloadedToChunk));
    assert!(fixture.world.entities.load().is_empty());
    let chunk = fixture
        .world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    let saved = chunk.data.lock().unwrap().clone();
    assert_eq!(saved.len(), 1);
    let NbtTag::Compound(stack) = &saved[0].get_list("Items").unwrap()[0] else {
        panic!("chest boat saved a non-compound item")
    };
    assert_eq!(stack.get_string("id").unwrap(), "minecraft:diamond");
    assert_eq!(stack.get_int("count"), Some(3));
    fixture.shutdown().await;
}

struct CancelFirstDestroy(AtomicUsize);
impl EventHandler<VehicleDestroyEvent> for CancelFirstDestroy {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut VehicleDestroyEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.0.fetch_add(1, Ordering::Relaxed) == 0;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_vehicle_destruction_releases_claim_for_next_hit() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fixture.server.plugin_manager.register(
        Arc::new(CancelFirstDestroy(AtomicUsize::new(0))),
        EventPriority::Normal,
        true,
    );
    assert!(!boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK));
    assert!(boat.get_entity().is_alive());
    assert!(boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK));
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hoe_and_shovel_respect_blocking_intent_and_the_used_hand() {
    use crate::item::{
        ItemBehaviour,
        items::{hoe::HoeItem, shovel::ShovelItem},
    };
    use pumpkin_inventory::player::player_inventory::PlayerInventory;
    for (behaviour, item, expected) in [
        (
            &HoeItem as &dyn ItemBehaviour,
            &Item::IRON_HOE,
            &Block::FARMLAND,
        ),
        (
            &ShovelItem as &dyn ItemBehaviour,
            &Item::IRON_SHOVEL,
            &Block::DIRT_PATH,
        ),
    ] {
        let fixture = Fixture::new();
        let player = TestPlayer::new(&fixture.world);
        let pos = BlockPos::new(8, 64, 9);
        for (sneaking, hand, transforms) in [
            (false, Hand::Right, false),
            (true, Hand::Right, true),
            (false, Hand::Left, true),
        ] {
            fixture.world.set_block_state(
                &pos,
                Block::GRASS_BLOCK.default_state.id,
                BlockFlags::FORCE_STATE,
            );
            player.player.get_entity().set_sneaking(sneaking);
            player.player.inventory.set_stack(
                PlayerInventory::OFF_HAND_SLOT,
                ItemStack::new(1, &Item::SHIELD),
            );
            let mut stack = ItemStack::new(1, item);
            behaviour.use_on_block_with_hand(
                &mut stack,
                &player.player,
                pos,
                BlockDirection::Up,
                Vector3::new(0.5, 1.0, 0.5),
                &Block::GRASS_BLOCK,
                &fixture.server,
                hand,
            );
            assert_eq!(
                fixture.world.get_block(&pos),
                if transforms {
                    expected
                } else {
                    &Block::GRASS_BLOCK
                }
            );
            assert_eq!(stack.get_damage(), i32::from(transforms));
        }
        fixture.shutdown().await;
    }
}

struct CountRegistryVisits {
    entity: Entity,
    visits: AtomicUsize,
}
impl EntityBase for CountRegistryVisits {
    fn get_entity(&self) -> &Entity {
        self.visits.fetch_add(1, Ordering::Relaxed);
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn item_removal_visits_registry_only_for_publication() {
    let fixture = Fixture::new();
    let item = Arc::new(CountRegistryVisits {
        entity: Entity::new(
            fixture.world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::ITEM,
        ),
        visits: AtomicUsize::new(0),
    });
    fixture.world.add_entity_silent(item.clone());
    item.visits.store(0, Ordering::Relaxed);
    assert!(fixture.world.remove_entity(&item.entity));
    // Publication must retain/remove entries once; ordinary items need no concrete callback lookup.
    assert_eq!(item.visits.load(Ordering::Relaxed), 1);
    fixture.shutdown().await;
}
