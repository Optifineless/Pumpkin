use super::*;
use crate::{entity::r#type::from_type, world::spawn_test_support::Fixture};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use tracing_subscriber::{Layer, layer::SubscriberExt};

struct DuplicateWarnings(Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<String>>>);

#[derive(Default)]
struct WarningFields(String);

impl tracing::field::Visit for WarningFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        write!(self.0, "{}={value:?} ", field.name()).unwrap();
    }
}

impl<S: tracing::Subscriber> Layer<S> for DuplicateWarnings {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() == tracing::Level::WARN
            && event
                .metadata()
                .target()
                .ends_with("world::spawn_insertion")
        {
            self.0.fetch_add(1, Relaxed);
            let mut fields = WarningFields::default();
            event.record(&mut fields);
            self.1.lock().unwrap().push(fields.0);
        }
    }
}

#[tokio::test]
async fn command_silent_and_restored_insertions_keep_the_first_uuid() {
    let fixture = Fixture::new();
    let world = &fixture.world;
    let uuid = uuid::Uuid::new_v4();
    let existing = from_type(&EntityType::COW, Vector3::default(), world, uuid);
    assert!(world.spawn_entity(existing.clone()));
    let warnings = Arc::new(AtomicUsize::new(0));
    let details = Arc::new(std::sync::Mutex::new(Vec::new()));
    let subscriber =
        tracing_subscriber::registry().with(DuplicateWarnings(warnings.clone(), details.clone()));
    tracing::subscriber::with_default(subscriber, || {
        for path in 0..3 {
            let duplicate = from_type(&EntityType::CHICKEN, Vector3::default(), world, uuid);
            match path {
                0 => assert!(!world.spawn_entity(duplicate.clone())),
                1 => world.add_entity_silent(duplicate.clone()),
                _ => assert!(!world.insert_restored_riding_tree(&duplicate)),
            }
            assert_eq!(world.entities.load().len(), 1);
            assert!(Arc::ptr_eq(
                &world.get_entity_by_uuid(uuid).unwrap(),
                &existing
            ));
            assert!(
                world
                    .get_entity_by_id(duplicate.get_entity().entity_id)
                    .is_none()
            );
        }
    });
    assert_eq!(warnings.load(Relaxed), 3);
    for warning in details.lock().unwrap().iter() {
        assert!(warning.contains(&uuid.to_string()));
        assert!(warning.contains("chicken"));
    }
    fixture.finish().await;
}
