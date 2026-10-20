#![expect(
    clippy::unwrap_used,
    reason = "Real destruction regression inspects the concrete spawned entity and callback payloads"
)]

use super::b1020d_g06_test_support::Fixture;
use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::{entity_spawn::EntitySpawnEvent, item_spawn::ItemSpawnEvent},
    },
    server::Server,
};
use pumpkin_data::{damage::DamageType, entity::EntityType, item::Item};
use pumpkin_util::math::vector3::Vector3;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct SpawnPositions(Mutex<Vec<Vector3<f64>>>);

impl EventHandler<ItemSpawnEvent> for SpawnPositions {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(event.position);
        Box::pin(async {})
    }
}

impl EventHandler<EntitySpawnEvent> for SpawnPositions {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_type == EntityType::ITEM.id.to_string() {
            self.0.lock().unwrap().push(event.position);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g06_r2_boat_drop_and_spawn_events_use_exact_fractional_vehicle_position() {
    let fixture = Fixture::new();
    let vessel = fixture.boat(&EntityType::OAK_CHEST_BOAT);
    let position = Vector3::new(8.125, 64.375, 8.875);
    vessel.get_entity().set_pos(position);
    let positions = Arc::new(SpawnPositions::default());
    fixture.server.plugin_manager.register::<ItemSpawnEvent, _>(
        positions.clone(),
        EventPriority::Normal,
        true,
    );
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(positions.clone(), EventPriority::Normal, true);
    assert!(vessel.damage(vessel.as_ref(), 5.0, DamageType::GENERIC));
    assert_eq!(
        *positions.0.lock().unwrap(),
        vec![position, position],
        "Entity.spawnAtLocation uses the vehicle's exact coordinates for both spawn events"
    );
    let item = fixture
        .world
        .entities
        .load()
        .iter()
        .find(|entity| {
            entity.get_entity().entity_type == &EntityType::ITEM
                && !entity.get_entity().is_removed()
        })
        .cloned()
        .unwrap();
    assert_eq!(item.get_entity().pos.load(), position);
    fixture.assert_named_drop(&Item::OAK_CHEST_BOAT);
    fixture.finish().await;
}
