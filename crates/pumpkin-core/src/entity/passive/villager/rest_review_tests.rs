use super::behavior_tests::{bed, prime_acquisition, spawn_villager};
use super::*;
use crate::{
    block::blocks::{bed::test_support::PlayerFixture, doors::DoorBlock},
    entity::ai::{
        brain::memory::{GlobalPos, types},
        goal::Goal,
    },
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{DoubleBlockHalf, HorizontalFacing, OakDoorLikeProperties},
};
use pumpkin_util::math::vector3::Vector3;

fn set_home(
    villager: &VillagerEntity,
    dimension: &'static pumpkin_data::dimension::Dimension,
    pos: BlockPos,
) {
    villager
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .set(types::HOME, GlobalPos::new(dimension, pos));
}

#[tokio::test]
async fn production_goals_reach_home_32_blocks_through_door_sleep_and_wake() {
    let fixture = PlayerFixture::new();
    for x in 0..3 {
        let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
        terrain.x = x;
        if x == 1 {
            for z in 0..16 {
                for y in 64..68 {
                    terrain.set_block_state(1, y, z, Block::STONE.default_state);
                }
            }
            let mut door = OakDoorLikeProperties::default(&Block::OAK_DOOR);
            door.facing = HorizontalFacing::East;
            for (half, y) in [(DoubleBlockHalf::Lower, 64), (DoubleBlockHalf::Upper, 65)] {
                door.half = half;
                terrain.set_block_state(1, y, 5, door.to_state_id(&Block::OAK_DOOR).to_state());
            }
        }
        publish(&fixture.world, terrain);
    }
    let home = bed(&fixture.world, BlockPos::new(35, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    prime_acquisition(&villager);
    let server = fixture.world.server.upgrade().unwrap();
    let door = BlockPos::new(17, 64, 5);
    let mut opened = false;
    for tick in 1..=1000 {
        fixture.world.level_time.lock().unwrap().world_age = tick;
        villager.tick(villager.as_ref(), &server);
        assert!(
            villager
                .mob_entity
                .brain
                .lock()
                .unwrap()
                .get(types::DOORS_TO_CLOSE)
                .is_none_or(|doors| doors.len() <= 2)
        );
        opened |= DoorBlock::is_open(&fixture.world, &door);
        if villager.get_entity().pose.load() == EntityPose::Sleeping {
            break;
        }
    }
    assert_eq!(villager.get_home_pos(), Some(home));
    assert!(opened, "production goals never opened the door");
    assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
    assert!(!DoorBlock::is_open(&fixture.world, &door));
    fixture.world.level_time.lock().unwrap().time_of_day = 10;
    villager.tick(villager.as_ref(), &server);
    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    fixture.finish().await;
}

#[tokio::test]
async fn failed_five_home_candidates_do_not_hide_reachable_sixth() {
    let fixture = Fixture::new();
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    for x in 5..13 {
        for z in 1..10 {
            for y in 64..68 {
                terrain.set_block_state(x, y, z, Block::STONE.default_state);
            }
        }
    }
    publish(&fixture.world, terrain);
    for (x, z) in [(7, 3), (9, 3), (11, 3), (7, 6), (9, 6)] {
        bed(&fixture.world, BlockPos::new(x - 1, 64, z));
    }
    let home = bed(&fixture.world, BlockPos::new(13, 64, 13));
    let villager = spawn_villager(&fixture.world, Vector3::new(2.5, 64.0, 5.5));
    prime_acquisition(&villager);
    villager.update_home();
    assert_eq!(villager.get_home_pos(), None);
    // AcquirePoi's next batch runs after 20..39 ticks, before the failed markers' 40..79 retry.
    fixture.world.level_time.lock().unwrap().world_age = 39;
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    fixture.finish().await;
}

#[tokio::test]
async fn cold_restart_and_owner_unload_preserve_awake_home_reservation() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let original = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    prime_acquisition(&original);
    original.update_home();
    assert_eq!(original.get_home_pos(), Some(home));
    let mut saved = pumpkin_nbt::NbtCompound::new();
    EntityBase::write_nbt(original.as_ref(), &mut saved);
    fixture.world.remove_entity(original.as_ref());
    drop(original);
    // Re-register a replacement chunk without releasing the saved ticket.
    let state = fixture.world.get_block_state_id(&home);
    fixture
        .world
        .level
        .loaded_chunks
        .remove(&home.chunk_position());
    let replacement = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    replacement.set_block_absolute_y(9, 64, 5, state);
    assert!(fixture.world.available_homes(home).is_empty());
    drop(chunk);
    let fixture = fixture.restart().await;
    let replacement = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    replacement.set_block_absolute_y(9, 64, 5, state);
    assert!(
        fixture.world.available_homes(home).is_empty(),
        "refresh freed the disk ticket"
    );
    let rival = spawn_villager(&fixture.world, Vector3::new(7.5, 64.0, 5.5));
    prime_acquisition(&rival);
    rival.update_home();
    assert_eq!(rival.get_home_pos(), None);
    let loaded = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    EntityBase::read_nbt_non_mut(loaded.as_ref(), &saved);
    loaded.update_home();
    assert_eq!(loaded.get_home_pos(), Some(home));
    assert!(fixture.world.available_homes(home).is_empty());
    loaded.mob_entity.living_entity.health.store(0.0);
    loaded.update_home();
    assert_eq!(loaded.get_home_pos(), None);
    assert_eq!(fixture.world.available_homes(home), vec![home]);
    fixture.finish().await;
}

#[tokio::test]
async fn vanilla_home_memory_is_authoritative_and_dimension_checked() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let original = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    set_home(
        &original,
        &pumpkin_data::dimension::Dimension::OVERWORLD,
        home,
    );
    let mut saved = pumpkin_nbt::NbtCompound::new();
    EntityBase::write_nbt(original.as_ref(), &mut saved);
    for field in ["HomeX", "HomeY", "HomeZ", "sleeping_pos"] {
        saved.child_tags.remove(field);
    }
    let loaded = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    EntityBase::read_nbt_non_mut(loaded.as_ref(), &saved);
    assert_eq!(loaded.get_home_pos(), Some(home));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    set_home(
        &loaded,
        &pumpkin_data::dimension::Dimension::THE_NETHER,
        home,
    );
    loaded.rest_tick(1000);
    assert_eq!(loaded.get_home_pos(), None);
    assert!(loaded.get_entity().pose.load() == EntityPose::Standing);
    fixture.finish().await;
}

#[tokio::test]
async fn interrupted_rest_route_restarts_during_same_night() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(12, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    set_home(
        &villager,
        &pumpkin_data::dimension::Dimension::OVERWORLD,
        home,
    );
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let mut goal = rest::SleepAtHomeGoal::default();
    assert!(goal.can_start(villager.as_ref()));
    goal.start(villager.as_ref());
    villager.mob_entity.navigator.lock().unwrap().stop();
    assert!(!goal.should_continue(villager.as_ref()));
    goal.stop(villager.as_ref());
    let ready = (0..40).any(|_| goal.can_start(villager.as_ref()));
    assert!(ready);
    goal.start(villager.as_ref());
    assert!(!villager.mob_entity.navigator.lock().unwrap().is_idle());
    goal.stop(villager.as_ref());
    villager
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .set(types::CANT_REACH_WALK_TARGET_SINCE, 0);
    fixture.world.level_time.lock().unwrap().world_age = 1201;
    for _ in 0..40 {
        assert!(!goal.can_start(villager.as_ref()));
    }
    assert_eq!(villager.get_home_pos(), None);
    fixture.finish().await;
}

#[tokio::test]
async fn panic_prevents_sleep_and_wakes_resting_villager() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    set_home(
        &villager,
        &pumpkin_data::dimension::Dimension::OVERWORLD,
        home,
    );
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    villager.rest_tick(1000);
    assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
    let zombie = crate::entity::mob::zombie::zombie::ZombieEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(10.5, 64.0, 5.5),
        &EntityType::ZOMBIE,
    ));
    fixture.world.add_entity_silent(zombie);
    fixture.world.level_time.lock().unwrap().world_age = 1020;
    villager.golem_ai_step();
    villager.rest_tick(1020);
    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    villager.rest_tick(1200);
    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    fixture.finish().await;
}

#[tokio::test]
async fn pr102_p2_same_tick_wake_does_not_resleep() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    set_home(
        &villager,
        &pumpkin_data::dimension::Dimension::OVERWORLD,
        home,
    );
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    fixture.world.level_time.lock().unwrap().world_age = 1_000;

    villager.rest_tick(1_000);
    assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
    villager.get_entity().set_rotation(90.0, 0.0);
    assert!(villager.wake_up_if_sleeping_at(home));
    assert_eq!(villager.get_home_pos(), Some(home));
    assert!(villager.wants_to_sleep());
    let wake_position = villager.get_entity().pos.load();
    let wake_distance_squared = home
        .to_centered_f64()
        .squared_distance_to_vec(&wake_position);
    assert!(
        wake_distance_squared < 4.0,
        "wake position {wake_position:?} is outside the sleep range of {home:?}"
    );
    villager.rest_tick(1_000);

    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    fixture.finish().await;
}

struct DoorEvents(std::sync::Mutex<Vec<String>>);
impl crate::plugin::EventHandler<crate::plugin::api::events::world::generic_game::GenericGameEvent>
    for DoorEvents
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::api::events::world::generic_game::GenericGameEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        self.0.lock().unwrap().push(event.event_key.clone());
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn door_transitions_emit_exactly_one_open_and_close_event() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let server = fixture.world.server.upgrade().unwrap();
    let events = Arc::new(DoorEvents(std::sync::Mutex::new(Vec::new())));
    server
        .plugin_manager
        .register::<crate::plugin::api::events::world::generic_game::GenericGameEvent, _>(
            events.clone(),
            crate::plugin::EventPriority::Normal,
            true,
        );
    let pos = BlockPos::new(5, 64, 5);
    fixture.world.set_block_state(
        &pos,
        Block::OAK_DOOR.default_state.id,
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    DoorBlock::set_open(&fixture.world, &pos, true);
    DoorBlock::set_open(&fixture.world, &pos, true);
    DoorBlock::set_open(&fixture.world, &pos, false);
    DoorBlock::set_open(&fixture.world, &pos, false);
    assert_eq!(
        *events.0.lock().unwrap(),
        vec![
            pumpkin_data::game_event::GameEvent::BlockOpen.name(),
            pumpkin_data::game_event::GameEvent::BlockClose.name()
        ]
    );
    fixture.finish().await;
}
