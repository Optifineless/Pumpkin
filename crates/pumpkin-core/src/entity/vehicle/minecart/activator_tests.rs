use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority, vehicle::vehicle_exit::VehicleExitEvent,
};
use crate::{
    net::java::combat_test_support::TestPlayer, server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::biome::Biome;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::time::Duration;

struct ExitHandler {
    cart: Arc<MinecartEntity>,
    cancelled: AtomicBool,
    unlocked_calls: AtomicU32,
}

impl EventHandler<VehicleExitEvent> for ExitHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut VehicleExitEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.vehicle_id, self.cart.get_entity().entity_id);
        assert!(self.cart.get_entity().passengers.try_lock().is_ok());
        self.unlocked_calls.fetch_add(1, Ordering::Relaxed);
        event.cancelled = self.cancelled.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

fn activator_rail(world: &Arc<crate::world::World>, powered: bool) {
    // PoweredRailBlock.updateState recomputes power when a rail is placed.
    let support = if powered {
        &Block::REDSTONE_BLOCK
    } else {
        &Block::STONE
    };
    world.set_block_state(
        &BlockPos::new(8, 63, 8),
        support.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut properties = PoweredRailLikeProperties::default(&Block::ACTIVATOR_RAIL);
    properties.powered = powered;
    world.set_block_state(
        &BlockPos::new(8, 64, 8),
        properties.to_state_id(&Block::ACTIVATOR_RAIL),
        BlockFlags::FORCE_STATE,
    );
}

fn tick_with_timeout(cart: Arc<dyn EntityBase>, server: Arc<Server>) {
    // A native thread makes a synchronous mutex deadlock fail on timeout without
    // making Tokio runtime shutdown wait forever for a blocked spawn_blocking task.
    let (completed, result) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    let worker = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        cart.tick(cart.as_ref(), &server);
        completed.send(()).unwrap();
    });
    result.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn powered_activator_rail_ejects_without_locking_passengers_twice() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    check_nonrideable_minecart_behavior(&world, &server);
    activator_rail(&world, false);
    let fixture = TestPlayer::new(&world);
    let passenger = fixture.player.clone();
    passenger.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    let cart = Arc::new(MinecartEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::MINECART,
    )));
    world.entities.store(Arc::new(vec![cart.clone()]));
    cart.get_entity()
        .add_passenger(cart.clone(), passenger.clone());
    cart.tick(cart.as_ref(), &server);
    assert!(cart.get_entity().has_passenger(passenger.entity_id()));
    activator_rail(&world, true);
    let handler = Arc::new(ExitHandler {
        cart: cart.clone(),
        cancelled: AtomicBool::new(true),
        unlocked_calls: AtomicU32::new(0),
    });
    server.plugin_manager.register::<VehicleExitEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );
    tick_with_timeout(cart.clone(), server.clone());
    assert!(cart.get_entity().has_passenger(passenger.entity_id()));
    assert!(passenger.get_entity().has_vehicle());
    assert!(passenger.awaiting_teleport.lock().unwrap().is_none());
    handler.cancelled.store(false, Ordering::Relaxed);
    tick_with_timeout(cart.clone(), server);
    assert_eq!(handler.unlocked_calls.load(Ordering::Relaxed), 2);
    assert!(!cart.get_entity().has_passengers());
    assert!(!passenger.get_entity().has_vehicle());
    assert_eq!(
        passenger
            .get_entity()
            .riding_cooldown
            .load(Ordering::Relaxed),
        60
    );
    assert!(passenger.awaiting_teleport.lock().unwrap().is_some());
    assert!(!fixture.client().is_closed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activator_rail_ejects_passengers_loaded_from_existing_chunk_nbt() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    // Entity.saveWithoutId / EntityType.loadPassengersRecursive: vanilla and older
    // fork saves embed the riding entity tree in the root's Passengers list.
    let position = NbtTag::List(vec![
        NbtTag::Double(8.5),
        NbtTag::Double(64.0),
        NbtTag::Double(8.5),
    ]);
    let mut passenger_nbt = NbtCompound::new();
    passenger_nbt.put_string("id", "minecraft:pig".into());
    passenger_nbt.put("Pos", position.clone());
    let mut cart_nbt = NbtCompound::new();
    cart_nbt.put_string("id", "minecraft:minecart".into());
    cart_nbt.put("Pos", position);
    cart_nbt.put(
        "Passengers",
        NbtTag::List(vec![NbtTag::Compound(passenger_nbt)]),
    );
    let chunk = world
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    *chunk.data.lock().unwrap() = vec![cart_nbt];
    world.make_chunk_entities_live(&chunk, None);
    let cart = world
        .entities
        .load()
        .iter()
        .find(|entity| entity.get_entity().entity_type.id == EntityType::MINECART.id)
        .unwrap()
        .clone();
    let passenger = cart.get_entity().passengers.lock().unwrap()[0].clone();
    assert!(passenger.get_entity().has_vehicle());
    assert!(cart.cast_any().is::<MinecartEntity>());
    assert_eq!(cart.get_entity().pos.load(), Vector3::new(8.5, 64.0, 8.5));
    activator_rail(&world, true);
    let (rail, state) = world.get_block_and_state_id(&BlockPos::new(8, 64, 8));
    assert_eq!(rail.id, Block::ACTIVATOR_RAIL.id);
    assert!(PoweredRailLikeProperties::from_state_id(state).powered);
    tick_with_timeout(cart.clone(), server);
    assert!(!cart.get_entity().has_passengers());
    assert!(!passenger.get_entity().has_vehicle());
    assert_eq!(
        passenger
            .get_entity()
            .riding_cooldown
            .load(Ordering::Relaxed),
        60
    );
}

fn check_nonrideable_minecart_behavior(world: &Arc<crate::world::World>, server: &Server) {
    let fixture = TestPlayer::new(world);
    let passenger = &fixture.player;
    for entity_type in [
        &EntityType::CHEST_MINECART,
        &EntityType::FURNACE_MINECART,
        &EntityType::HOPPER_MINECART,
        &EntityType::TNT_MINECART,
        &EntityType::COMMAND_BLOCK_MINECART,
        &EntityType::SPAWNER_MINECART,
    ] {
        let cart = Arc::new(MinecartEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            entity_type,
        )));
        world.entities.store(Arc::new(vec![cart.clone()]));
        cart.get_entity()
            .add_passenger(cart.clone(), passenger.clone());
        activator_rail(world, true);
        cart.tick(cart.as_ref(), server);
        assert!(cart.get_entity().has_passenger(passenger.entity_id()));
        let mut nbt = NbtCompound::new();
        cart.write_custom_nbt(&mut nbt);
        if entity_type.id == EntityType::TNT_MINECART.id {
            assert_eq!(nbt.get_int("fuse"), Some(79));
        } else if entity_type.id == EntityType::HOPPER_MINECART.id {
            assert_eq!(nbt.get_bool("Enabled"), Some(false));
            activator_rail(world, false);
            cart.tick(cart.as_ref(), server);
            cart.write_custom_nbt(&mut nbt);
            assert_eq!(nbt.get_bool("Enabled"), Some(true));
        }
        cart.get_entity()
            .remove_passenger_sync(passenger.entity_id());
    }
}
