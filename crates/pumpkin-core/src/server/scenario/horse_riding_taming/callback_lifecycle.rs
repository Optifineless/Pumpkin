use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority, api::events::entity::entity_tame::EntityTameEvent,
};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::Mutex;

struct LifecycleCallback {
    target: Arc<dyn EntityBase>,
    destination: Option<Arc<World>>,
    captured: Mutex<Option<NbtCompound>>,
}

impl EventHandler<EntityTameEvent> for LifecycleCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityTameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(20), async {
                if let Some(destination) = &self.destination {
                    if let Some(vehicle) = event.owner.get_entity().get_vehicle() {
                        vehicle
                            .get_entity()
                            .remove_passenger_before_teleport(event.owner.entity_id());
                    }
                    assert!(!event.owner.get_entity().has_vehicle());
                    event
                        .owner
                        .teleport_world(destination.clone(), SPAWN, None, None)
                        .await;
                } else {
                    let world = self.target.get_entity().world.load_full();
                    world.save().await;
                    let storage = world
                        .level
                        .get_entity_chunk(self.target.get_entity().chunk_pos.load())
                        .await
                        .unwrap();
                    *self.captured.lock().unwrap() = storage
                        .data
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|nbt| {
                            nbt.get_uuid("UUID") == Some(self.target.get_entity().entity_uuid)
                        })
                        .cloned();
                    event.cancelled = true;
                }
            })
            .await
            .unwrap();
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_horses_s2_pending_tame_save_and_real_dimension_transfer() {
    for transfer in [false, true] {
        run(async {
            let scenario = Scenario::new().await;
            let id = Uuid::new_v4();
            let owner = scenario.join(id, "HorseLifecycle").await;
            let world = owner.player.world();
            let entity = spawn(&world);
            let uuid = entity.get_entity().entity_uuid;
            horse(&entity).set_temper(5);
            interact(&scenario, &owner, &entity);
            let handler = Arc::new(LifecycleCallback {
                target: entity.clone(),
                destination: transfer.then(|| scenario.world(&Dimension::THE_NETHER)),
                captured: Mutex::new(None),
            });
            scenario
                .server
                .plugin_manager
                .register::<EntityTameEvent, _>(handler.clone(), EventPriority::Normal, true);
            owner.packets();
            script(&entity, 0, 4);
            opportunity(&scenario, &entity).await;
            assert!(!horse(&entity).is_tame());
            assert_eq!(horse(&entity).get_owner(), None);
            assert_eq!(horse(&entity).get_temper(), 5);
            assert!(!tamed_advancement(&owner));
            assert!(statuses(&owner, &entity).is_empty());
            if transfer {
                assert_eq!(owner.player.world().dimension, Dimension::THE_NETHER);
                assert!(!owner.player.get_entity().has_vehicle());
                owner.loaded(&scenario.server);
                scenario
                    .until("callback dimension departure watcher tasks", || {
                        scenario.server.tasks.is_empty() && owner.is_idle()
                    })
                    .await;
                scenario.ticks(1).await;
                assert!(
                    entity.get_entity().is_removed(),
                    "callback-triggered departure did not retire the old horse"
                );
                assert!(world.get_entity_by_uuid(uuid).is_none());
                assert!(!entity.get_entity().has_passengers());
                scenario.stop().await;
            } else {
                let saved = handler.captured.lock().unwrap().take().unwrap();
                assert_eq!(
                    saved.get_bool("Tame"),
                    Some(false),
                    "save captured an uncommitted success"
                );
                assert_eq!(saved.get_uuid("Owner"), None);
                assert_eq!(saved.get_int("Temper"), Some(5));
                let scenario = scenario.restart().await;
                let owner = scenario.join(id, "HorseLifecycle").await;
                let loaded = owner.player.world().get_entity_by_uuid(uuid).unwrap();
                assert!(!horse(&loaded).is_tame());
                assert_eq!(horse(&loaded).get_temper(), 5);
                assert!(!horse(&loaded).has_flag(FLAG_STANDING));
                assert!(
                    statuses(&owner, &loaded).is_empty(),
                    "pending callback replayed feedback after restart"
                );
                interact(&scenario, &owner, &loaded);
                script(&loaded, 0, 4);
                opportunity(&scenario, &loaded).await;
                assert!(
                    horse(&loaded).is_tame(),
                    "pending token survived restart and blocked a fresh attempt"
                );
                scenario.stop().await;
            }
        })
        .await;
    }
}
