use super::*;
use pumpkin_protocol::{packet::MultiVersionJavaPacket, ser::NetworkReadExt};

use crate::{
    entity::{death_test_world::DeathTestWorld, shearable::Shearable},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::entity_transform::EntityTransformEvent,
    },
    server::Server,
};
use pumpkin_data::{item::Item, item_stack::ItemStack, sound::SoundCategory};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct ConversionControl {
    cancel: AtomicBool,
    calls: AtomicUsize,
}
impl EventHandler<EntityTransformEvent> for ConversionControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityTransformEvent,
    ) -> BoxFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        event.cancelled = self.cancel.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_mooshroom_conversion_has_no_effects() {
    let fixture = DeathTestWorld::new().await;
    let mob = fixture.mob(&EntityType::MOOSHROOM);
    let mut observer = crate::net::java::combat_test_support::TestPlayer::new(&fixture.world());
    let player = observer.player.clone();
    let control = Arc::new(ConversionControl {
        cancel: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityTransformEvent, _>(control.clone(), EventPriority::Normal, true);
    let mut tool = ItemStack::new(1, &Item::SHEARS);
    observer.take_packets();
    assert!(!mob.interact(&player, &mut tool));
    let sound_id = pumpkin_protocol::java::client::play::CSoundEffect::to_id(
        pumpkin_data::packet::CURRENT_MC_VERSION,
    );
    assert!(
        observer
            .take_packets()
            .iter()
            .all(|packet| packet.as_ref().get_var_int().unwrap().0 != sound_id)
    );
    assert!(!mob.get_entity().is_removed());
    assert_eq!(tool.get_damage(), 0);
    assert_eq!(fixture.world().entities.load().len(), 1);
    control.cancel.store(false, Ordering::Relaxed);
    assert!(mob.interact(&player, &mut tool));
    assert_eq!(control.calls.load(Ordering::Relaxed), 2);
    assert!(mob.get_entity().is_removed());
    assert_eq!(tool.get_damage(), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mooshroom_conversion_preserves_common_state_and_mounts() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let mob = fixture.mob(&EntityType::MOOSHROOM);
    let mushroom = mob.cast_any().downcast_ref::<MooshroomEntity>().unwrap();
    let raw = mob.get_entity();
    raw.set_pos(Vector3::new(8.5, 64.0, 8.5));
    raw.set_rotation(30.0, 15.0);
    raw.velocity.store(Vector3::new(0.2, 0.3, 0.4));
    raw.set_fall_flying(true);
    raw.set_custom_data("test", "conversion", pumpkin_nbt::tag::NbtTag::Int(27));
    raw.set_custom_name(pumpkin_util::text::TextComponent::text("Named cow"));
    mushroom.mob_entity.set_left_handed(true);
    mushroom.mob_entity.set_no_ai(true);
    mushroom
        .mob_entity
        .living_entity
        .hurt_cooldown
        .store(17, Ordering::Relaxed);
    let passenger = fixture.mob(&EntityType::SHEEP);
    let vehicle = fixture.mob(&EntityType::COW);
    raw.add_passenger(mob.clone(), passenger.clone());
    vehicle
        .get_entity()
        .add_passenger(vehicle.clone(), mob.clone());
    passenger
        .get_entity()
        .riding_cooldown
        .store(41, Ordering::Relaxed);
    assert!(mushroom.shear(SoundCategory::Players, &ItemStack::new(1, &Item::SHEARS)));
    let entities = world.entities.load_full();
    let cow = entities
        .iter()
        .find(|e| {
            e.get_entity().entity_type == &EntityType::COW
                && e.get_entity().entity_id != vehicle.get_entity().entity_id
        })
        .unwrap();
    let converted = cow.get_mob().unwrap().get_mob_entity();
    assert!(converted.is_left_handed());
    assert!(converted.is_no_ai());
    assert_eq!(
        converted
            .living_entity
            .hurt_cooldown
            .load(Ordering::Relaxed),
        17
    );
    assert!(cow.get_entity().is_fall_flying());
    assert_eq!(
        cow.get_entity().get_custom_data("test", "conversion"),
        Some(pumpkin_nbt::tag::NbtTag::Int(27))
    );
    assert_eq!(
        cow.get_entity().velocity.load(),
        Vector3::new(0.2, 0.3, 0.4)
    );
    assert_eq!(cow.get_entity().yaw.load(), 30.0);
    assert_eq!(
        passenger
            .get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_id,
        cow.get_entity().entity_id
    );
    assert_eq!(
        cow.get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_id,
        vehicle.get_entity().entity_id
    );
    assert_eq!(
        passenger
            .get_entity()
            .riding_cooldown
            .load(Ordering::Relaxed),
        0
    );
    fixture.server.shutdown().await;
}
