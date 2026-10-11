use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::{ChunkEntityData, io::DirtyFlag};
use uuid::Uuid;

use super::{PortalProcessor, PortalType};
use crate::entity::{EntityBase, death_test_world::DeathTestWorld};

async fn load_saved_travelers(
    fixture: &DeathTestWorld,
) -> (Arc<dyn EntityBase>, Arc<dyn EntityBase>) {
    let world = fixture.world();
    let player = fixture.player("saved-portal-cooldown");
    let position = Vector2::new(0, 0);
    let mut records = Vec::new();
    let mut ids = Vec::new();
    // Older fork builds and vanilla both save an int PortalCooldown: zero or remaining ticks.
    // A failed trip in an older build left zero; an active cooldown must also survive loading.
    for remaining in [0, 137] {
        let id = Uuid::new_v4();
        let entity = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(8.5, 80.0, 9.0),
            &world,
            id,
        );
        let mut nbt = NbtCompound::new();
        entity.write_nbt(&mut nbt);
        nbt.put_int("PortalCooldown", remaining);
        records.push(nbt);
        ids.push(id);
    }
    world
        .level
        .write_entity_chunks(vec![(
            position,
            Arc::new(ChunkEntityData {
                x: position.x,
                z: position.y,
                data: Mutex::new(records),
                live: AtomicBool::new(false),
                dirty: DirtyFlag::new(true),
            }),
        )])
        .await;
    world.level.drain_entity_storage().await.unwrap();
    assert!(world.level.get_entity_chunk_sync(&position).is_none());
    world.level.mark_chunks_as_newly_watched(&[position]).await;
    // Real player-watched chunk loading: disk read, activation, and entity restoration.
    world.spawn_world_entity_chunks(player, vec![position]);
    tokio::time::timeout(Duration::from_secs(20), async {
        while ids.iter().any(|id| world.get_entity_by_uuid(*id).is_none()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let traveler = world.get_entity_by_uuid(ids[0]).unwrap();
    let cooling = world.get_entity_by_uuid(ids[1]).unwrap();
    (traveler, cooling)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_cooldowns_load_through_entity_chunks_and_failed_trips_do_not_retry() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let (traveler, cooling) = load_saved_travelers(&fixture).await;
    assert_eq!(traveler.get_entity().portal_cooldown.load(Relaxed), 0);
    assert_eq!(cooling.get_entity().portal_cooldown.load(Relaxed), 137);
    cooling.get_entity().tick(cooling.as_ref(), &fixture.server);
    assert_eq!(cooling.get_entity().portal_cooldown.load(Relaxed), 136);

    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    // A real chunk await fails after shutdown, so destination resolution returns None.
    destination.level.cancel_token.cancel();
    let entity = traveler.get_entity();
    *entity.portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::Nether,
        BlockPos::new(8, 80, 8),
        destination.clone(),
    ));
    entity.tick(traveler.as_ref(), &fixture.server);
    tokio::time::timeout(Duration::from_secs(20), async {
        while entity.portal_travel_pending_for_test() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&entity.world.load_full(), &world));
    assert!(entity.portal_cooldown.load(Relaxed) > 0);
    for _ in 0..3 {
        entity.try_use_portal(destination.clone(), BlockPos::new(8, 80, 8));
        entity.tick(traveler.as_ref(), &fixture.server);
        assert!(!entity.portal_travel_pending_for_test());
        assert!(entity.portal_cooldown.load(Relaxed) > 0);
    }
    assert!(
        destination
            .level
            .chunk_loading
            .lock()
            .unwrap()
            .ticket
            .is_empty()
    );
    fixture.server.shutdown().await;
}
