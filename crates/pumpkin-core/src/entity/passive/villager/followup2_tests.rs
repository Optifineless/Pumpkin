use super::behavior_tests::{bed, prime_acquisition, spawn_villager};
use super::*;
use crate::{
    block::blocks::bed::test_support::PlayerFixture,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{Block, biome::Biome, block_properties::WhiteBedLikeProperties};
use pumpkin_util::math::vector3::Vector3;

#[tokio::test]
async fn followup2_home_survives_tick_before_chunk_publication() {
    for sleeping in [false, true] {
        let fixture = Fixture::new();
        publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
        let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
        let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
        prime_acquisition(&villager);
        villager.update_home();
        if sleeping {
            fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
            villager.rest_tick(0);
            assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
        }
        let mut saved = pumpkin_nbt::NbtCompound::new();
        EntityBase::write_nbt(villager.as_ref(), &mut saved);
        let fixture = fixture.restart().await;
        fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
        let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
        EntityBase::read_nbt_non_mut(villager.as_ref(), &saved);
        villager.villager_mob_tick();
        assert_eq!(villager.get_home_pos(), Some(home));
        if sleeping {
            assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
        }
        let owner: Arc<dyn EntityBase> = villager.clone();
        assert!(fixture.world.claim_home(home, Arc::downgrade(&owner)));
        assert_eq!(
            fixture
                .world
                .portal_poi
                .lock()
                .unwrap()
                .free_tickets(&home, "minecraft:home"),
            Some(0)
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn followup2_panic_is_cached_until_the_next_sensor_scan() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let zombie = crate::entity::mob::zombie::zombie::ZombieEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(10.5, 64.0, 5.5),
        &EntityType::ZOMBIE,
    ));
    fixture.world.add_entity_silent(zombie.clone());
    fixture.world.level_time.lock().unwrap().world_age = 20;
    villager.golem_ai_step();
    assert!(!villager.wants_to_sleep());
    fixture.world.remove_entity(zombie.as_ref());
    assert!(
        !villager.wants_to_sleep(),
        "REST performed a new entity scan between sensor ticks"
    );
    fixture.world.level_time.lock().unwrap().world_age = 40;
    villager.golem_ai_step();
    assert!(villager.wants_to_sleep());
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_occupied_player_bed_forgets_home_at_validation_cadence() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    prime_acquisition(&villager);
    villager.update_home();
    villager.update_home(); // First validation.
    let mut props = WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home));
    props.occupied = true;
    fixture.world.set_block_state(
        &home,
        props.to_state_id(&Block::RED_BED),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    fixture.world.level_time.lock().unwrap().world_age = 19;
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    fixture.world.level_time.lock().unwrap().world_age = 20;
    villager.update_home();
    assert_eq!(villager.get_home_pos(), None);
    assert_eq!(
        fixture
            .world
            .portal_poi
            .lock()
            .unwrap()
            .free_tickets(&home, "minecraft:home"),
        Some(1)
    );
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_far_occupied_home_is_not_validated() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    prime_acquisition(&villager);
    villager.update_home();
    let mut props = WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home));
    props.occupied = true;
    fixture.world.set_block_state(
        &home,
        props.to_state_id(&Block::RED_BED),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    villager.get_entity().set_pos(Vector3::new(40.5, 64.0, 5.5));
    fixture.world.level_time.lock().unwrap().world_age = 20;
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_acquiring_home_broadcasts_happy_particles_once() {
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let mut client = crate::net::java::combat_test_support::TestPlayer::new(&fixture.world);
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    fixture
        .world
        .entity_tracker
        .add_entity(&(villager.clone() as Arc<dyn EntityBase>), &fixture.world);
    fixture
        .world
        .entity_tracker
        .get_tracked_entity(villager.get_entity().entity_id)
        .unwrap()
        .seen_by
        .insert(client.player.gameprofile.id);
    prime_acquisition(&villager);
    villager.update_home();
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    let expected = client
        .client()
        .serialize_packet(&pumpkin_protocol::java::client::play::CEntityStatus::new(
            villager.get_entity().entity_id,
            14,
        ))
        .unwrap();
    assert_eq!(
        client
            .take_packets()
            .iter()
            .filter(|packet| **packet == expected)
            .count(),
        1
    );
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_batch_path_selects_the_shortest_reachable_target() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let villager = spawn_villager(&fixture.world, Vector3::new(2.5, 64.0, 5.5));
    let targets = [BlockPos::new(13, 64, 5), BlockPos::new(6, 64, 5)];
    let path = villager
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .create_path_to_targets(&villager.mob_entity, &targets, 1)
        .unwrap();
    assert!(path.can_reach());
    assert_eq!(path.get_target(), targets[1]);
    fixture.finish().await;
}

#[tokio::test]
async fn followup2_door_close_moves_memory_without_copying_its_allocation() {
    use crate::entity::ai::{
        brain::memory::{GlobalPos, types},
        goal::interact_with_door::close_doors,
    };
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let villager = spawn_villager(&fixture.world, Vector3::new(5.5, 64.0, 5.5));
    let door = BlockPos::new(5, 64, 5);
    let memory = std::iter::once(GlobalPos::new(
        &pumpkin_data::dimension::Dimension::OVERWORLD,
        door,
    ))
    .collect::<rustc_hash::FxHashSet<_>>();
    villager
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .set(types::DOORS_TO_CLOSE, memory);
    let address = || {
        let brain = villager.mob_entity.brain.lock().unwrap();
        std::ptr::from_ref(
            brain
                .get(types::DOORS_TO_CLOSE)
                .unwrap()
                .iter()
                .next()
                .unwrap(),
        ) as usize
    };
    let previous = address();
    close_doors(villager.as_ref(), Some(door), None);
    assert_eq!(address(), previous);
    fixture.finish().await;
}

#[derive(Default)]
struct DoorSources(std::sync::Mutex<Vec<(String, Option<uuid::Uuid>)>>);
impl crate::plugin::EventHandler<crate::plugin::api::events::world::generic_game::GenericGameEvent>
    for DoorSources
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::api::events::world::generic_game::GenericGameEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push((event.event_key.clone(), event.source_entity));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup3_villager_door_events_carry_the_actor_for_both_goals() {
    use crate::entity::ai::{
        brain::memory::types,
        goal::{Goal, door_interact::DoorInteractGoal, interact_with_door::InteractWithDoorGoal},
    };
    let fixture = PlayerFixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let server = fixture.world.server.upgrade().unwrap();
    let events = Arc::new(DoorSources::default());
    server
        .plugin_manager
        .register::<crate::plugin::api::events::world::generic_game::GenericGameEvent, _>(
            events.clone(),
            crate::plugin::EventPriority::Normal,
            true,
        );
    let door = BlockPos::new(5, 64, 5);
    fixture.world.set_block_state(
        &door,
        Block::OAK_DOOR.default_state.id,
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    let villager = spawn_villager(&fixture.world, Vector3::new(5.5, 64.0, 5.5));
    let path = crate::entity::ai::pathfinder::path::Path::new(
        vec![
            crate::entity::ai::pathfinder::node::Node::new(door),
            crate::entity::ai::pathfinder::node::Node::new(BlockPos::new(
                door.0.x + 1,
                door.0.y,
                door.0.z,
            )),
        ],
        door,
        true,
    );
    villager.mob_entity.navigator.lock().unwrap().move_to_path(
        Some(path),
        0.5,
        &villager.mob_entity.living_entity,
    );
    InteractWithDoorGoal::default().tick(villager.as_ref());
    assert!(
        villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .get(types::DOORS_TO_CLOSE)
            .is_some()
    );
    crate::entity::ai::goal::interact_with_door::close_doors(villager.as_ref(), None, None);
    let mut door_goal = DoorInteractGoal::new();
    door_goal.door_pos = door;
    door_goal.has_door = true;
    for open in [true, true, false, false] {
        door_goal.set_open(villager.as_ref(), open);
    }
    let (event_count, correct_sources) = {
        let events = events.0.lock().unwrap();
        (
            events.len(),
            events
                .iter()
                .all(|(_, source)| *source == Some(villager.get_entity().entity_uuid)),
        )
    };
    assert_eq!(event_count, 4);
    assert!(correct_sources);
    fixture.finish().await;
}

#[tokio::test]
async fn followup3_home_validation_runs_only_during_rest() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    prime_acquisition(&villager);
    villager.update_home();
    let mut props = WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home));
    props.occupied = true;
    fixture.world.set_block_state(
        &home,
        props.to_state_id(&Block::RED_BED),
        pumpkin_world::world::BlockFlags::NOTIFY_ALL,
    );
    fixture.world.level_time.lock().unwrap().world_age = 20;
    fixture.world.level_time.lock().unwrap().time_of_day = 6_000;
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    villager.sensed_panic.store(true, Ordering::Relaxed);
    villager.update_home();
    assert_eq!(villager.get_home_pos(), Some(home));
    villager.sensed_panic.store(false, Ordering::Relaxed);
    villager.update_home();
    assert_eq!(villager.get_home_pos(), None);
    fixture.finish().await;
}

#[tokio::test]
async fn followup3_reached_home_claim_race_does_not_delay_other_candidates() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let homes = [
        bed(&fixture.world, BlockPos::new(8, 64, 5)),
        bed(&fixture.world, BlockPos::new(12, 64, 5)),
    ];
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    let path = villager
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .create_path_to_targets(&villager.mob_entity, &homes, 1)
        .unwrap();
    assert!(path.can_reach());
    let target = path.get_target();
    let rival = spawn_villager(&fixture.world, Vector3::new(7.5, 64.0, 5.5));
    let rival_owner: Arc<dyn EntityBase> = rival.clone();
    assert!(
        fixture
            .world
            .claim_home(target, Arc::downgrade(&rival_owner))
    );
    let owner: Arc<dyn EntityBase> = villager.clone();
    let candidates = {
        let mut acquisition = villager.home_acquisition.lock().unwrap();
        villager.acquire_home_from_path(
            Some(path),
            &homes,
            Arc::downgrade(&owner),
            &mut acquisition,
        );
        assert_eq!(villager.get_home_pos(), None);
        acquisition.candidates(homes.to_vec(), 20, &mut rand::rng())
    };
    assert_eq!(
        candidates, homes,
        "a lost ticket race must not create unreachable-path retry markers"
    );
    fixture.finish().await;
}
