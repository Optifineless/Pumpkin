//! Serialize a real cow through the public `EntityBase` boundary, then publish real /tp watcher changes.
use super::*;
use crate::entity::{Entity, living::LivingEntity, mob::Mob};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::sync::{LockClass, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

struct SerializationCompletion;
impl LockClass for SerializationCompletion {
    const NAME: &'static str = "scenario serialization completion";
    const LEVEL: u8 = 30;
}
struct ObservedMob {
    mob: Arc<dyn EntityBase>,
    completion: Mutex<Option<Box<dyn FnOnce() + Send>>, SerializationCompletion>,
    snapshots: AtomicUsize,
}
impl EntityBase for ObservedMob {
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get_entity(&self) -> &Entity {
        self.mob.get_entity()
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        self.mob.get_living_entity()
    }
    fn get_mob(&self) -> Option<&dyn Mob> {
        self.mob.get_mob()
    }
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.mob.write_nbt(nbt);
        self.snapshots.fetch_add(1, Ordering::Relaxed);
        let completion = self.completion.lock().unwrap().take();
        if let Some(completion) = completion {
            completion();
        }
    }
}
fn teleport(server: &Arc<Server>, input: &str) {
    let source = crate::command::CommandSender::Console
        .into_source(server)
        .with_silent();
    server
        .command_dispatcher
        .load()
        .execute_input(input, &source)
        .unwrap();
}
fn assert_live_once(
    world: &World,
    mob: &ObservedMob,
    storage: &pumpkin_world::level::SyncEntityChunk,
) {
    assert!(
        !mob.get_entity().is_removed(),
        "stale unload retired the rewatched cow"
    );
    assert_eq!(
        world
            .entities
            .load()
            .iter()
            .filter(|entity| entity.get_entity().entity_uuid == mob.get_entity().entity_uuid)
            .count(),
        1,
        "rewatch must retain exactly one registry instance"
    );
    assert!(
        std::ptr::eq(
            world
                .get_entity_by_uuid(mob.get_entity().entity_uuid)
                .unwrap()
                .get_entity(),
            mob.get_entity()
        ),
        "rewatch replaced the original live cow"
    );
    assert!(storage.live.load(Ordering::Acquire));
    assert!(
        storage.data.lock().unwrap().is_empty(),
        "cancelled retirement wrote a storage record"
    );
}

async fn settled(scenario: &Scenario, session: &Session, label: &str) {
    scenario
        .until(label, || {
            session.is_idle() && scenario.server.tasks.is_empty()
        })
        .await;
}
fn pending(world: &World, pos: Vector2<i32>) -> bool {
    world
        .level
        .pending_entity_unloads()
        .iter()
        .any(|request| request.position == pos)
}

fn observed_mob(world: &Arc<World>) -> Arc<ObservedMob> {
    let inner = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(10.5, 64.0, 10.5),
        world,
        Uuid::new_v4(),
    );
    inner
        .get_entity()
        .set_custom_name(pumpkin_util::text::TextComponent::text("RetirementWitness"));
    let mob = Arc::new(ObservedMob {
        mob: inner,
        completion: Mutex::new(None),
        snapshots: AtomicUsize::new(0),
    });
    world.add_entity_silent(mob.clone());
    mob
}

#[tokio::test]
async fn rewatch_before_commit_keeps_entities_and_cancels_pending_unload() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "UnloadOwner").await;
        let viewer = scenario.join(Uuid::new_v4(), "UnloadViewer").await;
        let world = owner.player.world();
        let pos = Vector2::new(0, 0);
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        scenario.command("tp UnloadViewer 163.5 64 3.5");
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "viewer departure").await;
        let mob = observed_mob(&world);
        let id = mob.get_entity().entity_uuid;
        let storage = world.level.get_entity_chunk_sync(&pos).unwrap();
        assert!(storage.data.lock().unwrap().is_empty());
        let server = scenario.server.clone();
        *mob.completion.lock().unwrap() = Some(Box::new(move || {
            teleport(&server, "tp UnloadViewer 3.5 64 3.5");
        }));
        scenario.command("tp UnloadOwner 163.5 64 3.5");
        owner.loaded(&scenario.server);
        settled(&scenario, &owner, "unload request publication").await;
        assert!(pending(&world, pos));
        assert!(
            world.level.is_chunk_loaded(&pos),
            "terrain must survive until retirement commits"
        );
        scenario.ticks(1).await;
        assert!(world.level.is_chunk_watched(&pos));
        assert_live_once(&world, &mob, &storage);
        assert!(!pending(&world, pos));
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "rewatch completion").await;
        scenario.ticks(1).await;
        // ABA negative control: a second watch/unwatch makes a fresh hidden generation while
        // the previous generation's serialized request is still in flight on this tick.
        let server = scenario.server.clone();
        *mob.completion.lock().unwrap() = Some(Box::new(move || {
            teleport(&server, "tp UnloadViewer 3.5 64 3.5");
            teleport(&server, "tp UnloadViewer 163.5 64 3.5");
        }));
        scenario.command("tp UnloadViewer 163.5 64 3.5");
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "second unload request").await;
        scenario.ticks(1).await;
        assert!(!world.level.is_chunk_watched(&pos));
        assert_live_once(&world, &mob, &storage);
        assert!(pending(&world, pos));
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "ABA watcher completion").await;
        scenario.ticks(1).await;
        assert!(mob.get_entity().is_removed());
        assert!(world.get_entity_by_uuid(id).is_none());
        assert!(!storage.live.load(Ordering::Acquire));
        assert_eq!(storage.data.lock().unwrap().len(), 1);
        assert!(!pending(&world, pos));
        let saved_name = storage.data.lock().unwrap()[0]
            .get("CustomName")
            .cloned()
            .unwrap();
        let snapshots = mob.snapshots.load(Ordering::Relaxed);
        assert_eq!(
            snapshots, 3,
            "expected two cancelled snapshots and one retirement"
        );
        scenario.ticks(2).await;
        assert_eq!(
            mob.snapshots.load(Ordering::Relaxed),
            snapshots,
            "retirement ran twice"
        );
        let scenario = scenario.restart().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario.join(Uuid::new_v4(), "RetirementReload").await;
        assert_eq!(
            world
                .entities
                .load()
                .iter()
                .filter(|entity| entity.get_entity().entity_uuid == id)
                .count(),
            1,
            "disk reload must restore exactly one cow"
        );
        let mut reloaded = NbtCompound::new();
        world
            .get_entity_by_uuid(id)
            .unwrap()
            .write_nbt(&mut reloaded);
        assert_eq!(reloaded.get("CustomName"), Some(&saved_name));
        scenario.stop().await;
    })
    .await;
}
