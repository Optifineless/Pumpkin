#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry regressions use bounded fixture setup and inspect admitted state"
)]

use super::b1020d_g06_test_support::{CHUNK, DEADLINE, Fixture, NAME, POS, drops};
use crate::{
    entity::{EntityBase, r#type::from_type},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::{entity_spawn::EntitySpawnEvent, item_spawn::ItemSpawnEvent},
    },
    server::{Server, combat_test_support},
    world::{World, portal::NetherPortal, spawn_test_support},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::HorizontalAxis, damage::DamageType,
    data_component_impl::CustomNameImpl, dimension::Dimension, entity::EntityType, item::Item,
};
use pumpkin_protocol::{VarInt, java::server::play::SInteract};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn board(fixture: &Fixture, vessel: &dyn EntityBase) {
    fixture.player.client().handle_interact(
        &fixture.player.player,
        &SInteract {
            entity_id: VarInt(vessel.get_entity().entity_id),
            r#type: VarInt(0),
            target_position: None,
            hand: Some(VarInt(0)),
            sneaking: false,
        },
        &fixture.server,
    );
    assert!(!fixture.player.client().is_closed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_surface_water_boat_accepts_boarding_after_sixty_world_ticks() {
    let fixture = Fixture::new();
    fixture.fill_contact(&Block::WATER);
    let vessel = fixture.boat(&EntityType::OAK_BOAT);
    vessel.get_entity().set_pos(Vector3::new(8.5, 64.6, 8.5));
    fixture.tick(30);
    fixture.tick(30);
    assert_eq!(vessel.get_entity().age.load(Ordering::Relaxed), 60);
    assert!(vessel.get_entity().touching_water.load(Ordering::Relaxed));
    board(&fixture, vessel.as_ref());
    assert!(
        fixture.player.player.get_entity().has_vehicle(),
        "normal partly submerged surface boat must remain boardable after sixty ticks"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_submerged_boat_rejects_boarding_and_resets_on_surfacing() {
    let fixture = Fixture::new();
    fixture.fill_contact(&Block::WATER);
    let vessel = fixture.boat(&EntityType::BAMBOO_RAFT);
    fixture.tick(30);
    fixture.tick(30);
    board(&fixture, vessel.as_ref());
    assert!(!fixture.player.player.get_entity().has_vehicle());
    vessel.get_entity().set_pos(Vector3::new(8.5, 64.6, 8.5));
    fixture.tick(1);
    board(&fixture, vessel.as_ref());
    assert!(
        fixture.player.player.get_entity().has_vehicle(),
        "AbstractBoat.tick resets the underwater timer immediately at the surface"
    );
    fixture.finish().await;
}

pub(super) fn portal_frame(world: &Arc<World>, origin: BlockPos) {
    // The complete possible exit footprint is resident before the real async lookup.
    for x in (origin.0.x - 22) >> 4..=(origin.0.x + 22) >> 4 {
        for z in (origin.0.z - 22) >> 4..=(origin.0.z + 22) >> 4 {
            combat_test_support::publish_empty_chunk(world, Vector2::new(x, z));
        }
    }
    NetherPortal::build_portal_frame(world, origin, HorizontalAxis::X, false);
    assert!(
        NetherPortal::get_on_axis(world, &origin, HorizontalAxis::X)
            .unwrap()
            .was_already_valid()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_portal_transfers_concrete_boat_and_nonplayer_riding_tree() {
    let fixture = Fixture::new();
    let destination = fixture.other_world(Dimension::THE_NETHER);
    portal_frame(&fixture.world, BlockPos::new(800, 64, 8));
    portal_frame(&destination, BlockPos::new(100, 64, 1));
    let vessel = fixture.boat(&EntityType::BAMBOO_CHEST_RAFT);
    vessel.get_entity().set_pos(Vector3::new(801.0, 64.0, 8.5));
    fixture
        .player
        .player
        .get_entity()
        .set_pos(Vector3::new(801.0, 64.0, 12.5));
    let source_storage = tokio::time::timeout(
        DEADLINE,
        fixture
            .world
            .level
            .get_entity_chunk(vessel.get_entity().chunk_pos.load()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        fixture
            .world
            .make_chunk_entities_live(&source_storage, None)
    );
    tokio::time::timeout(
        DEADLINE,
        fixture
            .world
            .level
            .mark_chunks_as_newly_watched(&[vessel.get_entity().chunk_pos.load()]),
    )
    .await
    .unwrap();
    let passenger = from_type(
        &EntityType::PIG,
        vessel.get_entity().pos.load(),
        &fixture.world,
        uuid::Uuid::new_v4(),
    );
    fixture.world.spawn_entity_non_save(passenger.clone());
    vessel
        .get_entity()
        .add_passenger(vessel.clone(), passenger.clone());
    assert!(passenger.get_entity().has_vehicle());
    tokio::time::timeout(DEADLINE, fixture.world.save())
        .await
        .unwrap();
    fixture.tick(2);
    assert_eq!(vessel.get_entity().age.load(Ordering::Relaxed), 2);
    tokio::time::timeout(
        DEADLINE,
        combat_test_support::wait_for_tasks(&fixture.server),
    )
    .await
    .expect("actual portal job must finish within its deadline");
    let restored = assert_portal_arrival(&fixture, &destination, &vessel, &passenger);
    tokio::time::timeout(DEADLINE, fixture.world.save())
        .await
        .unwrap();
    tokio::time::timeout(DEADLINE, destination.save())
        .await
        .unwrap();
    assert!(
        source_storage
            .data
            .lock()
            .unwrap()
            .iter()
            .all(|record| { record.get_uuid("UUID") != Some(vessel.get_entity().entity_uuid) }),
        "source save must withdraw the transferred root"
    );
    let position = restored.get_entity().chunk_pos.load();
    let storage = tokio::time::timeout(DEADLINE, destination.level.get_entity_chunk(position))
        .await
        .unwrap()
        .unwrap();
    let records = storage.data.lock().unwrap().clone();
    assert_eq!(
        records
            .iter()
            .filter(|record| { record.get_uuid("UUID") == Some(restored.get_entity().entity_uuid) })
            .count(),
        1,
        "destination save must own the transferred root exactly once"
    );
    let root = records
        .iter()
        .find(|record| record.get_uuid("UUID") == Some(restored.get_entity().entity_uuid))
        .unwrap();
    assert_eq!(root.get_list("Passengers").unwrap().len(), 1);
    fixture.finish().await;
}

fn assert_portal_arrival(
    fixture: &Fixture,
    destination: &Arc<World>,
    vessel: &Arc<dyn EntityBase>,
    passenger: &Arc<dyn EntityBase>,
) -> Arc<dyn EntityBase> {
    let restored = destination
        .get_entity_by_uuid(vessel.get_entity().entity_uuid)
        .expect("completed Nether travel must publish the boat in the destination registry");
    assert!(vessel.get_entity().is_removed());
    assert!(
        fixture
            .world
            .get_entity_by_uuid(vessel.get_entity().entity_uuid)
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &restored.get_entity().world.load_full(),
        destination
    ));
    assert!(!std::ptr::eq(restored.get_entity(), vessel.get_entity()));
    assert_eq!(
        restored.get_entity().last_sent_pos.load(),
        restored.get_entity().pos.load()
    );
    assert_eq!(
        restored.get_entity().entity_type,
        &EntityType::BAMBOO_CHEST_RAFT
    );
    assert_eq!(
        *restored.get_entity().custom_name.load_full(),
        Some(TextComponent::text(NAME))
    );
    assert!((restored.get_entity().pos.load().x - 101.0).abs() < 2.0);
    let rider = destination
        .get_entity_by_uuid(passenger.get_entity().entity_uuid)
        .expect("nonplayer passenger must transfer with the boat");
    assert!(passenger.get_entity().is_alive());
    assert!(Arc::ptr_eq(&rider, passenger));
    assert_eq!(
        rider.get_entity().last_sent_pos.load(),
        rider.get_entity().pos.load()
    );
    assert!(
        fixture
            .world
            .get_entity_by_uuid(passenger.get_entity().entity_uuid)
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &rider.get_entity().world.load_full(),
        destination
    ));
    assert_eq!(
        rider
            .get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_uuid,
        restored.get_entity().entity_uuid
    );
    restored
}

#[derive(Clone, Copy)]
enum SpawnStage {
    Item,
    Entity,
}

struct UnloadOnSpawn {
    world_name: String,
    stage: SpawnStage,
    invoked: AtomicBool,
}

impl UnloadOnSpawn {
    fn unload<'a>(&'a self, server: &'a Arc<Server>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !self.invoked.swap(true, Ordering::AcqRel) {
                tokio::time::timeout(DEADLINE, server.unload_world(&self.world_name))
                    .await
                    .expect("uncancelled spawn callback world unload must finish")
                    .unwrap();
            }
        })
    }
}

impl EventHandler<ItemSpawnEvent> for UnloadOnSpawn {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(self.stage, SpawnStage::Item) {
            self.unload(server)
        } else {
            Box::pin(async {})
        }
    }
}

impl EventHandler<EntitySpawnEvent> for UnloadOnSpawn {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(self.stage, SpawnStage::Entity)
            && event.entity_type == EntityType::ITEM.id.to_string()
        {
            self.unload(server)
        } else {
            Box::pin(async {})
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_uncancelled_spawn_callback_unload_reload_conserves_boat_item() {
    spawn_callback_retirement(SpawnStage::Entity).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_uncancelled_item_spawn_callback_unload_reload_conserves_boat_item() {
    spawn_callback_retirement(SpawnStage::Item).await;
}

async fn spawn_callback_retirement(stage: SpawnStage) {
    let fixture = Fixture::new();
    let retired = fixture
        .server
        .create_world("boat_retirement".into(), Dimension::OVERWORLD);
    spawn_test_support::publish(
        &retired,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    let storage = tokio::time::timeout(DEADLINE, retired.level.get_entity_chunk(CHUNK))
        .await
        .unwrap()
        .unwrap();
    assert!(retired.make_chunk_entities_live(&storage, None));
    let vessel = from_type(
        &EntityType::OAK_CHEST_BOAT,
        POS,
        &retired,
        uuid::Uuid::new_v4(),
    );
    vessel
        .get_entity()
        .set_custom_name(TextComponent::text(NAME));
    retired.spawn_entity_non_save(vessel.clone());
    let callback = Arc::new(UnloadOnSpawn {
        world_name: "boat_retirement".into(),
        stage,
        invoked: AtomicBool::new(false),
    });
    fixture.server.plugin_manager.register::<ItemSpawnEvent, _>(
        callback.clone(),
        EventPriority::Normal,
        true,
    );
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(callback.clone(), EventPriority::Normal, true);
    let _accepted = vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC);
    assert!(callback.invoked.load(Ordering::Acquire));
    assert!(retired.level.cancel_token.is_cancelled());
    assert!(
        !fixture
            .server
            .worlds
            .load()
            .iter()
            .any(|world| Arc::ptr_eq(world, &retired))
    );
    let reloaded = fixture
        .server
        .create_world("boat_retirement".into(), Dimension::OVERWORLD);
    let storage = tokio::time::timeout(DEADLINE, reloaded.level.get_entity_chunk(CHUNK))
        .await
        .unwrap()
        .unwrap();
    assert!(reloaded.make_chunk_entities_live(&storage, None));
    let boats = reloaded
        .entities
        .load()
        .iter()
        .filter(|entity| {
            entity.get_entity().entity_type == &EntityType::OAK_CHEST_BOAT
                && !entity.get_entity().is_removed()
                && *entity.get_entity().custom_name.load_full() == Some(TextComponent::text(NAME))
        })
        .count();
    let items = drops(&reloaded)
        .iter()
        .filter(|stack| {
            stack.item == &Item::OAK_CHEST_BOAT
                && stack.item_count == 1
                && stack
                    .get_data_component::<CustomNameImpl>()
                    .is_some_and(|component| component.name == TextComponent::text(NAME))
        })
        .count();
    assert_eq!(
        boats + items,
        1,
        "full named-world unload/reload must conserve one concrete boat obligation"
    );
    assert!(
        drops(&retired).is_empty(),
        "a retired world's registry must not receive late items"
    );
    retired.level.world_portal.store(Arc::new(None));
    reloaded.level.world_portal.store(Arc::new(None));
    fixture.finish().await;
}

struct CancelDrop {
    stage: SpawnStage,
    observed: AtomicUsize,
}

impl EventHandler<ItemSpawnEvent> for CancelDrop {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(self.stage, SpawnStage::Item) {
            self.observed.fetch_add(1, Ordering::Relaxed);
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

impl EventHandler<EntitySpawnEvent> for CancelDrop {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if matches!(self.stage, SpawnStage::Entity)
            && event.entity_type == EntityType::ITEM.id.to_string()
        {
            self.observed.fetch_add(1, Ordering::Relaxed);
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_spawn_cancellation_still_destroys_live_boat_without_item() {
    for stage in [SpawnStage::Item, SpawnStage::Entity] {
        let fixture = Fixture::new();
        let vessel = fixture.boat(&EntityType::OAK_BOAT);
        let callback = Arc::new(CancelDrop {
            stage,
            observed: AtomicUsize::new(0),
        });
        fixture.server.plugin_manager.register::<ItemSpawnEvent, _>(
            callback.clone(),
            EventPriority::Normal,
            true,
        );
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(callback.clone(), EventPriority::Normal, true);
        assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
        assert_eq!(callback.observed.load(Ordering::Relaxed), 1);
        assert!(vessel.get_entity().is_removed());
        assert!(fixture.drops().is_empty());
        fixture.finish().await;
    }
}
