use super::test_support::{DEPARTURE, Scenario};
use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        player::Player,
        projectile::{arrow::ArrowEntity, ownership::ProjectileState},
    },
    server::combat_test_support,
    world::World,
};
use pumpkin_data::{entity::EntityType, game_event::GameEvent};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering::Relaxed},
};

enum NativeBase {
    Arrow(Box<ArrowEntity>),
    Living(Box<LivingEntity>),
}

// Public, unsealed EntityBase getters have no purity contract. This native source
// enters through spawn_entity and uses production transfer/reset entry points.
struct NativeSource {
    base: NativeBase,
    source: Arc<World>,
    destination: Arc<World>,
    armed: AtomicU8,
    ran: AtomicU8,
}

impl NativeSource {
    fn entity(&self) -> &Entity {
        match &self.base {
            NativeBase::Arrow(arrow) => &arrow.entity,
            NativeBase::Living(living) => &living.entity,
        }
    }
}

impl EntityBase for NativeSource {
    fn get_entity(&self) -> &Entity {
        if self.armed.compare_exchange(1, 0, Relaxed, Relaxed).is_ok() {
            let position = self.entity().pos.load();
            self.teleport(position, None, None, self.destination.clone());
            self.teleport(position, None, None, self.source.clone());
            self.ran.store(1, Relaxed);
        }
        self.entity()
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        match &self.base {
            NativeBase::Living(living) => Some(living),
            NativeBase::Arrow(_) => None,
        }
    }

    fn get_player(&self) -> Option<&Player> {
        if self.armed.compare_exchange(2, 0, Relaxed, Relaxed).is_ok() {
            if let NativeBase::Living(living) = &self.base {
                living.reset_state();
            }
            self.ran.store(2, Relaxed);
        }
        None
    }

    fn projectile_state(&self) -> Option<&ProjectileState> {
        match &self.base {
            NativeBase::Arrow(arrow) => Some(&arrow.projectile),
            NativeBase::Living(_) => None,
        }
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_native_getters_transfer_reentry() {
    for enabled in [true, false] {
        native_getter_case(enabled, 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_native_getters_life_reentry() {
    for enabled in [true, false] {
        native_getter_case(enabled, 2);
    }
}

fn native_getter_case(enabled: bool, mode: u8) {
    let s = Scenario::new(enabled);
    let other_dir = tempfile::tempdir().unwrap();
    let destination = combat_test_support::world(&s.server, other_dir.path());
    s.server
        .worlds
        .store(Arc::new(vec![s.world.clone(), destination.clone()]));
    let base = if mode == 1 {
        NativeBase::Arrow(Box::new(ArrowEntity::new(
            Entity::new(s.world.clone(), DEPARTURE, &EntityType::ARROW),
            None,
        )))
    } else {
        NativeBase::Living(Box::new(LivingEntity::new(Entity::new(
            s.world.clone(),
            DEPARTURE,
            &EntityType::COW,
        ))))
    };
    let source = Arc::new(NativeSource {
        base,
        source: s.world.clone(),
        destination,
        armed: AtomicU8::new(0),
        ran: AtomicU8::new(0),
    });
    assert!(s.world.spawn_entity(source.clone()));
    let original = s
        .world
        .game_event_source_membership(source.entity().entity_id)
        .unwrap();
    let life = source
        .get_living_entity()
        .map(LivingEntity::damage_lifecycle);
    let once = AtomicBool::new(false);
    let arm = source.clone();
    s.capture(Some(Box::new(move |_, _| {
        if !once.swap(true, Relaxed) {
            arm.armed.store(mode, Relaxed);
        }
    })));
    s.world.game_event_item_interact_finish(source.entity());
    // OFF must never invoke any getter for event capture/revalidation.
    assert_eq!(source.ran.load(Relaxed), if enabled { mode } else { 0 });
    source.armed.store(0, Relaxed);
    assert!(!source.entity().is_removed());
    assert!(Arc::ptr_eq(&source.entity().world.load_full(), &s.world));
    let current = s
        .world
        .game_event_source_membership(source.entity().entity_id)
        .unwrap();
    assert_eq!(Arc::ptr_eq(&original, &current), !enabled || mode == 2);
    if let Some(living) = source.get_living_entity() {
        assert_eq!(
            living.damage_lifecycle(),
            life.unwrap() + u64::from(enabled)
        );
    }
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.plugins
            .iter()
            .filter(|e| e.key == "item_interact_finish")
            .count(),
        1
    );
    assert_eq!(
        sink.forwarded
            .iter()
            .filter(|e| e.event == GameEvent::ItemInteractFinish)
            .count(),
        0,
        "native getter reentry must reject stale admission or life after all getters"
    );
}
