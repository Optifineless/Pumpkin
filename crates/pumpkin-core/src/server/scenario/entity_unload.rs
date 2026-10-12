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
    inner.get_mob().unwrap().get_mob_entity().set_no_ai(true);
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
        scenario.command("tp UnloadOwner 163.5 64 3.5");
        owner.loaded(&scenario.server);
        settled(&scenario, &owner, "unload request publication").await;
        let stale = world
            .level
            .pending_entity_unloads()
            .into_iter()
            .find(|r| r.position == pos)
            .unwrap();
        assert!(world.level.is_chunk_loaded(&pos));
        scenario.command("tp UnloadViewer 3.5 64 3.5");
        assert!(
            !world
                .level
                .commit_entity_unload(&stale, || panic!("rewatch admitted stale commit"))
        );
        assert_live_once(&world, &mob, &storage);
        assert!(!pending(&world, pos));
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "rewatch completion").await;
        scenario.ticks(1).await;
        scenario.command("tp UnloadViewer 163.5 64 3.5");
        viewer.loaded(&scenario.server);
        settled(&scenario, &viewer, "ABA watcher completion").await;
        assert!(
            !world
                .level
                .commit_entity_unload(&stale, || panic!("ABA admitted stale commit"))
        );
        assert_live_once(&world, &mob, &storage);
        assert!(pending(&world, pos));
        scenario.ticks(1).await;
        assert!(mob.get_entity().is_removed());
        assert!(world.get_entity_by_uuid(id).is_none());
        assert!(!storage.live.load(Ordering::Acquire));
        assert_eq!(storage.data.lock().unwrap().len(), 1);
        assert!(!pending(&world, pos));
        assert_eq!(mob.snapshots.load(Ordering::Relaxed), 1);
        scenario.ticks(2).await;
        assert_eq!(
            mob.snapshots.load(Ordering::Relaxed),
            1,
            "retirement ran twice"
        );
        let saved_name = storage.data.lock().unwrap()[0]
            .get("CustomName")
            .cloned()
            .unwrap();
        let scenario = scenario.restart().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario.join(Uuid::new_v4(), "RetirementReload").await;
        assert_eq!(
            world
                .entities
                .load()
                .iter()
                .filter(|e| e.get_entity().entity_uuid == id)
                .count(),
            1
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

fn rejected_command(server: &Arc<Server>, input: &str) {
    let source = crate::command::CommandSender::Console
        .into_source(server)
        .with_silent();
    assert!(
        server
            .command_dispatcher
            .load()
            .execute_input(input, &source)
            .is_err(),
        "retired entity accepted {input}"
    );
}

fn reject_retired_writes(
    server: &Arc<Server>,
    root: &Arc<ObservedMob>,
    rider: &Arc<dyn EntityBase>,
) {
    let entity = root.get_entity();
    let id = entity.entity_uuid;
    rejected_command(server, &format!("tp {id} 12.5 64 12.5"));
    rejected_command(
        server,
        &format!("ride {id} mount {}", rider.get_entity().entity_uuid),
    );
    assert!(
        entity.is_removed(),
        "serialization began before irrevocable retirement"
    );
    assert!(entity.removal_reason.load() == Some(crate::entity::RemovalReason::UnloadedToChunk));
    let position = entity.pos.load();
    let yaw = entity.yaw.load();
    let world = entity.world.load_full();
    world.remove_entity(root.as_ref());
    assert!(
        entity.removal_reason.load() == Some(crate::entity::RemovalReason::UnloadedToChunk),
        "later removal overwrote the committed reason"
    );
    entity.teleport(
        Vector3::new(12.5, 64.0, 12.5),
        Some(90.0),
        Some(45.0),
        &world,
    );
    entity.set_pos(Vector3::new(13.5, 64.0, 13.5));
    entity.set_world(server.get_world_from_dimension(&Dimension::THE_NETHER));
    root.get_living_entity().unwrap().set_health(1.0);
    entity.remove_passenger(rider.get_entity().entity_id);
    entity.remove_passenger_sync(rider.get_entity().entity_id);
    rider
        .get_entity()
        .add_passenger(rider.clone(), root.clone());
    assert_eq!(entity.pos.load(), position, "late position write landed");
    assert_eq!(entity.yaw.load(), yaw, "late teleport rotation landed");
    assert!(
        Arc::ptr_eq(&entity.world.load_full(), &world),
        "late world write landed"
    );
    assert_eq!(
        root.get_living_entity().unwrap().health.load(),
        8.0,
        "late health write landed"
    );
    assert_eq!(
        entity.passengers.lock().unwrap().len(),
        1,
        "late dismount landed"
    );
    assert!(entity.get_vehicle().is_none(), "late mount landed");
}

#[tokio::test]
async fn retirement_rejects_late_writes_and_reloads_complete_riding_tree() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "RetirementOwner").await;
        let world = owner.player.world();
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        let root = observed_mob(&world);
        root.get_living_entity().unwrap().set_health(8.0);
        let rider = crate::entity::r#type::from_type(
            &EntityType::COW,
            Vector3::new(32.5, 65.0, 10.5),
            &world,
            Uuid::new_v4(),
        );
        world.add_entity_silent(rider.clone());
        let root_id = root.get_entity().entity_uuid;
        let rider_id = rider.get_entity().entity_uuid;
        scenario.command(&format!("ride {rider_id} mount {root_id}"));
        let storage = world
            .level
            .get_entity_chunk_sync(&Vector2::new(0, 0))
            .unwrap();
        let server = scenario.server.clone();
        let witness = root.clone();
        let passenger = rider.clone();
        *root.completion.lock().unwrap() = Some(Box::new(move || {
            reject_retired_writes(&server, &witness, &passenger);
            teleport(&server, "tp RetirementOwner 3.5 64 3.5");
        }));
        scenario.command("tp RetirementOwner 163.5 64 3.5");
        owner.loaded(&scenario.server);
        settled(&scenario, &owner, "retirement request").await;
        scenario.ticks(1).await;
        let record = storage.data.lock().unwrap()[0].clone();
        assert_eq!(
            record.get_list("Passengers").unwrap().len(),
            1,
            "unloaded rider was omitted"
        );
        let pumpkin_nbt::tag::NbtTag::Compound(rider_record) =
            &record.get_list("Passengers").unwrap()[0]
        else {
            panic!("rider record is not a compound");
        };
        assert_eq!(
            rider_record.get_list("Pos").unwrap()[0],
            pumpkin_nbt::tag::NbtTag::Double(10.5)
        );
        assert_eq!(
            rider_record.get_list("Pos").unwrap()[2],
            pumpkin_nbt::tag::NbtTag::Double(10.5)
        );
        assert!(
            root.get_entity().passengers.lock().unwrap().is_empty(),
            "terminal teardown left links"
        );
        assert!(rider.get_entity().get_vehicle().is_none());
        owner.loaded(&scenario.server);
        settled(&scenario, &owner, "postcommit rewatch").await;
        scenario.ticks(1).await;
        assert!(
            world.get_entity_by_uuid(root_id).is_some(),
            "postcommit rewatch lost root"
        );
        assert!(
            world.get_entity_by_uuid(rider_id).is_some(),
            "postcommit rewatch lost rider"
        );
        let scenario = scenario.restart().await;
        let world = scenario.world(&Dimension::OVERWORLD);
        scenario.join(Uuid::new_v4(), "TreeReload").await;
        let reloaded = world.get_entity_by_uuid(root_id).unwrap();
        assert_eq!(
            reloaded.get_entity().pos.load(),
            Vector3::new(10.5, 64.0, 10.5)
        );
        assert_eq!(reloaded.get_living_entity().unwrap().health.load(), 8.0);
        assert_eq!(
            world
                .entities
                .load()
                .iter()
                .filter(|e| [root_id, rider_id].contains(&e.get_entity().entity_uuid))
                .count(),
            2
        );
        assert_eq!(
            world
                .get_entity_by_uuid(rider_id)
                .unwrap()
                .get_entity()
                .get_vehicle()
                .unwrap()
                .get_entity()
                .entity_uuid,
            root_id
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn forceload_between_last_unwatch_and_entity_phase_keeps_original_mob_live() {
    run(async {
        let scenario = Scenario::new().await;
        let owner = scenario.join(Uuid::new_v4(), "ForceOwner").await;
        let world = owner.player.world();
        let pos = Vector2::new(0, 0);
        scenario.prepare_area(&world, Vector2::new(10, 0)).await;
        let mob = observed_mob(&world);
        let storage = world.level.get_entity_chunk_sync(&pos).unwrap();
        scenario.command("tp ForceOwner 163.5 64 3.5");
        owner.loaded(&scenario.server);
        settled(&scenario, &owner, "last unwatch").await;
        assert!(pending(&world, pos));
        // No await/poll between command and entity phase: saver task has not published anything.
        scenario.command("forceload add 0 0");
        scenario.server.tick();
        assert_live_once(&world, &mob, &storage);
        assert!(
            world.level.is_chunk_watched(&pos),
            "force ticket did not publish watch synchronously"
        );
        assert_eq!(mob.snapshots.load(Ordering::Relaxed), 0);
        scenario.command("forceload remove 0 0");
        assert!(!world.level.is_chunk_watched(&pos));
        scenario.command("forceload add 0 0");
        scenario.command("forceload remove 0 0");
        scenario.command("forceload add 0 0");
        assert!(world.level.is_chunk_watched(&pos));
        settled(&scenario, &owner, "ordered force saver tasks").await;
        assert_live_once(&world, &mob, &storage);
        scenario.command("forceload remove 0 0");
        settled(&scenario, &owner, "final unforce").await;
        scenario.ticks(1).await;
        assert!(mob.get_entity().is_removed());
        scenario.stop().await;
    })
    .await;
}
