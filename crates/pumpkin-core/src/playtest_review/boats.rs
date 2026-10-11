use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::item_spawn::ItemSpawnEvent, vehicle::vehicle_damage::VehicleDamageEvent,
        },
    },
};
use pumpkin_data::damage::DamageType;
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static HIT_RECEIVED_EVENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct CountVehicleEvents {
    gate: Barrier,
    damage: AtomicUsize,
    destroy: AtomicUsize,
}
impl EventHandler<VehicleDamageEvent> for CountVehicleEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleDamageEvent,
    ) -> BoxFuture<'a, ()> {
        HIT_RECEIVED_EVENT.set(true);
        self.damage.fetch_add(1, Ordering::Relaxed);
        self.gate.wait();
        Box::pin(async {})
    }
}
impl EventHandler<crate::plugin::api::events::vehicle::vehicle_destroy::VehicleDestroyEvent>
    for CountVehicleEvents
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut crate::plugin::api::events::vehicle::vehicle_destroy::VehicleDestroyEvent,
    ) -> BoxFuture<'a, ()> {
        self.destroy.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronized_lethal_boat_hits_emit_one_vehicle_item() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    let events = Arc::new(CountVehicleEvents {
        gate: Barrier::new(2),
        damage: AtomicUsize::new(0),
        destroy: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<VehicleDamageEvent, _>(events.clone(), EventPriority::Normal, true);
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::vehicle::vehicle_destroy::VehicleDestroyEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    let barrier = Arc::new(Barrier::new(2));
    let runtime = tokio::runtime::Handle::current();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let boat = boat.clone();
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            let events = events.clone();
            std::thread::spawn(move || {
                let _entered = runtime.enter();
                barrier.wait();
                assert!(boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK));
                // The first event cannot finish until the losing caller either enters it or returns.
                if !HIT_RECEIVED_EVENT.get() {
                    events.gate.wait();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(events.damage.load(Ordering::Relaxed), 1);
    assert_eq!(events.destroy.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 1);
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_chest_boat_scatters_contents_once_without_vehicle_item() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    boat.kill(boat.as_ref());
    boat.kill(boat.as_ref());
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unloading_chest_boat_preserves_contents_without_scattering() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    fixture
        .world
        .remove_entities_in_chunks([Vector2::new(0, 0)])
        .await;
    assert_eq!(fixture.drops(&Item::DIAMOND), 0);
    let mut saved = NbtCompound::new();
    boat.write_custom_nbt(&mut saved);
    assert_eq!(saved.get_list("Items").unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_boat_menu_closes_on_destruction_and_rejects_distant_use() {
    let fixture = Fixture::new();
    let opener = TestPlayer::new(&fixture.world);
    let attacker = TestPlayer::new(&fixture.world);
    fixture.world.players.store(Arc::new(vec![
        opener.player.clone(),
        attacker.player.clone(),
    ]));
    opener
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 8.0));
    opener.player.get_entity().set_sneaking(true);
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    assert!(boat.interact(&opener.player, &mut ItemStack::EMPTY.clone()));
    let menu = opener.player.current_screen_handler.lock().unwrap().clone();
    assert!(menu.lock().unwrap().can_use(opener.player.as_ref()));
    opener
        .player
        .get_entity()
        .set_pos(Vector3::new(24.0, 64.0, 8.0));
    assert!(!menu.lock().unwrap().can_use(opener.player.as_ref()));
    opener
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 8.0));
    boat.damage_with_context(
        boat.as_ref(),
        5.0,
        DamageType::PLAYER_ATTACK,
        None,
        Some(attacker.player.as_ref()),
        Some(attacker.player.as_ref()),
    );
    assert!(!menu.lock().unwrap().can_use(opener.player.as_ref()));
    opener.player.tick(&fixture.server);
    assert!(Arc::ptr_eq(
        &opener.player.current_screen_handler.lock().unwrap(),
        &(opener.player.player_screen_handler.clone()
            as pumpkin_inventory::screen_handler::SharedScreenHandler)
    ));
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_lava_contact_destroys_boat_and_preserves_fire_resistant_contents() {
    let fixture = Fixture::new();
    for x in 6..=10 {
        for y in 63..=65 {
            for z in 6..=10 {
                fixture.world.set_block_state(
                    &BlockPos::new(x, y, z),
                    Block::LAVA.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
        }
    }
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::NETHERITE_INGOT);
    for _ in 0..3 {
        boat.tick(boat.as_ref(), &fixture.server);
    }
    assert!(boat.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 1);
    assert_eq!(fixture.drops(&Item::NETHERITE_INGOT), 3);
    let contents: Vec<_> = fixture
        .world
        .entities
        .load()
        .iter()
        .filter(|e| {
            e.get_item_entity()
                .is_some_and(|i| i.get_item_stack().lock().unwrap().item == &Item::NETHERITE_INGOT)
        })
        .cloned()
        .collect();
    for item in contents {
        assert!(item.get_entity().is_fire_immune());
        for _ in 0..5 {
            item.tick(item.as_ref(), &fixture.server);
        }
        assert!(item.get_entity().is_alive());
    }
    fixture.shutdown().await;
}

struct CancelItems(AtomicUsize);
impl EventHandler<ItemSpawnEvent> for CancelItems {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ItemSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boat_and_container_drops_obey_item_spawn_cancellation() {
    let fixture = Fixture::new();
    let cancel = Arc::new(CancelItems(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register(cancel.clone(), EventPriority::Normal, true);
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK);
    assert_eq!(cancel.0.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.drops(&Item::DIAMOND), 0);
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_boats_allow_one_passenger_and_keep_mountable_interaction_pass() {
    let fixture = Fixture::new();
    let first = TestPlayer::new(&fixture.world);
    let second = TestPlayer::new(&fixture.world);
    fixture
        .world
        .players
        .store(Arc::new(vec![first.player.clone(), second.player.clone()]));
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    assert!(boat.interact(&first.player, &mut ItemStack::EMPTY.clone()));
    assert!(boat.interact(&second.player, &mut ItemStack::EMPTY.clone()));
    assert_eq!(boat.get_entity().passengers.lock().unwrap().len(), 1);
    assert!(!second.player.get_entity().has_vehicle());
    assert!(!Arc::ptr_eq(
        &second.player.current_screen_handler.lock().unwrap(),
        &(second.player.player_screen_handler.clone()
            as pumpkin_inventory::screen_handler::SharedScreenHandler)
    ));
    let other = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    // A player already riding another vehicle still receives Pass if the empty boat can mount.
    assert!(!other.interact(&first.player, &mut ItemStack::EMPTY.clone()));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_contents_have_zero_delay_and_vehicle_item_has_ten() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    fill_boat(boat.as_ref(), &Item::DIAMOND);
    boat.damage(boat.as_ref(), 5.0, DamageType::PLAYER_ATTACK);
    for item in fixture
        .world
        .entities
        .load()
        .iter()
        .filter_map(|e| e.get_item_entity())
    {
        let stack = item.get_item_stack().lock().unwrap();
        assert_eq!(
            item.get_pickup_delay(),
            if stack.item == &Item::DIAMOND { 0 } else { 10 }
        );
    }
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 1);
    fixture.shutdown().await;
}
