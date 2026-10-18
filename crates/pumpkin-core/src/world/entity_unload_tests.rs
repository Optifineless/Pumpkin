use super::*;
use crate::entity::{Entity, EntityBase, living::LivingEntity, r#type::from_type};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::{Arc, atomic::Ordering};

#[tokio::test]
async fn registry_cas_loss_never_marks_or_serializes_members() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let root = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    let rider = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 65.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(root.clone());
    world.add_entity_silent(rider.clone());
    root.get_entity().add_passenger(root.clone(), rider.clone());
    world.level.request_entity_unloads([pos]);
    let request = world
        .level
        .pending_entity_unloads()
        .into_iter()
        .find(|r| r.position == pos)
        .unwrap();
    let current = world.entities.load_full();
    // Real registry admission wins against the captured retirement incarnation.
    let newcomer = from_type(
        &EntityType::COW,
        Vector3::new(40.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(newcomer);
    assert!(
        world
            .commit_entity_retirement(
                &request,
                &current,
                Some(Arc::new(Vec::new())),
                &[root.get_entity(), rider.get_entity()]
            )
            .is_none()
    );
    for member in [&root, &rider] {
        assert!(!member.get_entity().is_removed());
        assert!(member.get_entity().removal_reason.load().is_none());
        assert!(!member.get_entity().removed.load(Ordering::Acquire));
        assert!(
            world
                .get_entity_by_uuid(member.get_entity().entity_uuid)
                .is_some()
        );
    }
    assert!(storage.live.load(Ordering::Acquire));
    assert!(storage.data.lock().unwrap().is_empty());
    assert!(
        world
            .level
            .pending_entity_unloads()
            .iter()
            .any(|r| r.position == pos)
    );
    world.process_entity_chunk_lifecycle();
    assert!(root.get_entity().is_removed());
    assert!(rider.get_entity().is_removed());
    assert_eq!(
        storage.data.lock().unwrap()[0]
            .get_list("Passengers")
            .unwrap()
            .len(),
        1
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_retries_after_entity_chunk_activation_restores_its_records() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let original = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );

    let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
    let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
    let paused = Arc::new(PausedEntitySave {
        inner: original.clone(),
        captured,
        resume: resume_writer,
        pause: std::sync::atomic::AtomicBool::new(true),
    });
    world.add_entity_silent(paused);

    // The wrapper blocks after World::save serializes its old entity registry.
    let save_world = world.clone();
    let save = tokio::spawn(async move { save_world.save().await });
    tokio::task::spawn_blocking(move || {
        wait_for_capture
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("entity save snapshot did not reach the barrier");
    })
    .await
    .unwrap();

    world.remove_entities_in_chunks([pos]).await;
    world.process_entity_chunk_lifecycle();
    assert!(original.get_entity().is_removed());
    assert!(!storage.live.load(Ordering::Acquire));
    assert_eq!(storage.data.lock().unwrap().len(), 1);

    // Reload storage after retirement, as the next watched chunk load does.
    let restored_storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&restored_storage, None);
    let restored = world
        .get_entity_by_uuid(original.get_entity().entity_uuid)
        .expect("entity activation did not restore the saved root");
    assert!(!restored.get_entity().is_removed());

    resume_capture.send(()).unwrap();
    save.await.unwrap();
    assert_eq!(
        restored_storage.data.lock().unwrap().len(),
        1,
        "stale ordinary save cleared records restored by chunk activation"
    );

    // Preserve the exact World::save image so shutdown cannot repair a stale result.
    world.entities.store(Arc::new(Vec::new()));
    restored_storage.live.store(false, Ordering::Release);
    let fixture = fixture.restart().await;
    let world = &fixture.world;
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    assert!(
        world
            .get_entity_by_uuid(original.get_entity().entity_uuid)
            .is_some(),
        "saved entity activation was lost after disk reload"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_waits_for_initial_entity_activation_before_replacing_stored_records() {
    let fixture = super::super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let (storage, root_uuid) = retire_cow_for_initial_activation(world).await;
    let other_pos = Vector2::new(2, 0);
    let other_storage = world.level.get_entity_chunk(other_pos).await.unwrap();
    world.make_chunk_entities_live(&other_storage, None);

    let (captured, mut capture_steps) = tokio::sync::mpsc::unbounded_channel();
    let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
    world.add_entity_silent(Arc::new(RepeatedEntitySave {
        inner: from_type(
            &EntityType::COW,
            Vector3::new(40.5, 64.0, 8.5),
            world,
            uuid::Uuid::new_v4(),
        ),
        captured,
        resume: resume_writer,
        captures: std::sync::atomic::AtomicUsize::new(0),
    }));

    let (entered, mut activation_entered) = tokio::sync::mpsc::unbounded_channel();
    let (resume_activation, activation_resume) = crossbeam::channel::bounded(1);
    *world.persistence_test_hooks.activation.lock().unwrap() = Some(Arc::new(move || {
        entered.send(()).unwrap();
        activation_resume
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
    }));
    let activating_world = world.clone();
    let activating_storage = storage.clone();
    let activation = std::thread::spawn(move || {
        activating_world.make_chunk_entities_live(&activating_storage, None);
    });
    let deadline = std::time::Duration::from_secs(30);
    assert_eq!(
        tokio::time::timeout(deadline, activation_entered.recv())
            .await
            .unwrap(),
        Some(())
    );

    // Start after live=true, before restore_entity_tree publishes the saved root.
    let saving_world = world.clone();
    let mut save = tokio::spawn(async move { saving_world.save().await });
    assert_eq!(
        tokio::time::timeout(deadline, capture_steps.recv())
            .await
            .unwrap(),
        Some(0)
    );
    resume_capture.send(()).unwrap();
    let progress = tokio::time::timeout(deadline, async {
        tokio::select! {
            step = capture_steps.recv() => (step, None),
            completed = &mut save => (None, Some(completed)),
        }
    })
    .await;
    let stored_during_activation = storage.data.lock().unwrap().len();
    let save_finished = matches!(&progress, Ok((_, Some(_))));

    // Release both callbacks before asserting, including when the negative control completes save.
    resume_activation.send(()).unwrap();
    activation.join().unwrap();
    resume_capture.send(()).unwrap();
    if !save_finished {
        save.await.unwrap();
    }
    let (retry_step, completion) = progress.expect("save neither retried nor completed");
    if let Some(completion) = completion {
        completion.unwrap();
    }
    assert_eq!(
        stored_during_activation, 1,
        "save cleared stored records before initial entity activation restored them"
    );
    assert_eq!(
        retry_step,
        Some(1),
        "save did not retry an active activation"
    );

    // Keep the exact ordinary-save result; shutdown must not repair a missing root.
    world.entities.store(Arc::new(Vec::new()));
    storage.live.store(false, Ordering::Release);
    other_storage.live.store(false, Ordering::Release);
    let fixture = fixture.restart().await;
    let world = &fixture.world;
    let storage = world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    world.make_chunk_entities_live(&storage, None);
    assert!(
        world.get_entity_by_uuid(root_uuid).is_some(),
        "initially activating entity was lost after disk reload"
    );
    fixture.finish().await;
}

async fn retire_cow_for_initial_activation(
    world: &Arc<World>,
) -> (pumpkin_world::level::SyncEntityChunk, uuid::Uuid) {
    let pos = Vector2::new(0, 0);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
    let root_uuid = uuid::Uuid::new_v4();
    let root = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        root_uuid,
    );
    world.add_entity_silent(root.clone());
    world.remove_entities_in_chunks([pos]).await;
    world.process_entity_chunk_lifecycle();
    assert!(root.get_entity().is_removed());
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    assert!(!storage.live.load(Ordering::Acquire));
    assert_eq!(storage.data.lock().unwrap().len(), 1);
    (storage, root_uuid)
}

struct RepeatedEntitySave {
    inner: Arc<dyn EntityBase>,
    captured: tokio::sync::mpsc::UnboundedSender<usize>,
    resume: crossbeam::channel::Receiver<()>,
    captures: std::sync::atomic::AtomicUsize,
}

impl EntityBase for RepeatedEntitySave {
    fn get_entity(&self) -> &Entity {
        self.inner.get_entity()
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        self.inner.get_living_entity()
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self.inner.cast_any()
    }

    fn write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.inner.write_nbt(nbt);
        let capture = self.captures.fetch_add(1, Ordering::AcqRel);
        if capture < 2 {
            self.captured.send(capture).unwrap();
            self.resume
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
        }
    }
}

struct PausedEntitySave {
    inner: Arc<dyn EntityBase>,
    captured: crossbeam::channel::Sender<()>,
    resume: crossbeam::channel::Receiver<()>,
    pause: std::sync::atomic::AtomicBool,
}

impl EntityBase for PausedEntitySave {
    fn get_entity(&self) -> &Entity {
        self.inner.get_entity()
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        self.inner.get_living_entity()
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self.inner.cast_any()
    }

    fn write_nbt(&self, nbt: &mut pumpkin_nbt::compound::NbtCompound) {
        self.inner.write_nbt(nbt);
        if self.pause.swap(false, Ordering::AcqRel) {
            self.captured.send(()).unwrap();
            self.resume
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
        }
    }
}
