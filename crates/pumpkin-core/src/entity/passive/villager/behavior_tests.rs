use super::*;
use crate::{
    block::blocks::doors::DoorBlock,
    entity::ai::{
        brain::memory::types,
        goal::{Goal, goal_selector::GoalSelector, interact_with_door::InteractWithDoorGoal},
        pathfinder::{node::PathType, pathfinding_context::PathfindingContext},
    },
    server::combat_test_support,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{
        BedPart, DoubleBlockHalf, HorizontalFacing, OakDoorLikeProperties, WhiteBedLikeProperties,
    },
};
use pumpkin_util::math::vector3::Vector3;
use pumpkin_world::world::BlockFlags;

pub(super) fn spawn_villager(world: &Arc<World>, pos: Vector3<f64>) -> Arc<VillagerEntity> {
    let villager = VillagerEntity::new(Entity::new(world.clone(), pos, &EntityType::VILLAGER));
    villager
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    world.add_entity_silent(villager.clone());
    villager
}

pub(super) fn bed(world: &Arc<World>, foot: BlockPos) -> BlockPos {
    let mut props = WhiteBedLikeProperties::default(&Block::RED_BED);
    props.facing = HorizontalFacing::East;
    props.part = BedPart::Foot;
    world.set_block_state(
        &foot,
        props.to_state_id(&Block::RED_BED),
        BlockFlags::NOTIFY_ALL,
    );
    foot.offset(props.facing.to_offset())
}

#[tokio::test]
async fn villager_claims_reachable_home_sleeps_and_wakes() {
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    fixture.world.level_time.lock().unwrap().world_age = 1_000;
    prime_acquisition(&villager);
    villager.villager_mob_tick();
    assert_eq!(villager.get_home_pos(), Some(home));
    let rival = spawn_villager(&fixture.world, Vector3::new(4.5, 64.0, 5.5));
    rival.update_home();
    assert_eq!(rival.get_home_pos(), None);
    let mut goal = rest::SleepAtHomeGoal::default();
    assert!(goal.can_start(villager.as_ref()));
    goal.start(villager.as_ref());
    assert!(!villager.mob_entity.navigator.lock().unwrap().is_idle());
    villager.get_entity().set_pos(Vector3::new(8.5, 64.0, 5.5));
    villager.rest_tick(1_020);
    assert!(villager.get_entity().pose.load() == EntityPose::Sleeping);
    assert_eq!(villager.get_entity().pos.load().y, 64.6875);
    assert!(
        WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home)).occupied
    );
    assert_eq!(
        villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .get(types::LAST_SLEPT),
        Some(&1_020)
    );
    assert!(!villager.can_tick_navigation());
    // The data-backed vanilla schedule leaves REST at tick 10, rather than dawn tick 0.
    fixture.world.level_time.lock().unwrap().time_of_day = 10;
    fixture.world.level_time.lock().unwrap().world_age = 2_000;
    villager.rest_tick(2_000);
    assert!(villager.get_entity().pose.load() == EntityPose::Standing);
    assert!(
        !WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home)).occupied
    );
    assert_eq!(
        villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .get(types::LAST_SLEPT),
        Some(&1_020)
    );
    assert_eq!(
        villager
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .get(types::LAST_WOKEN),
        Some(&2_000)
    );
    fixture.finish().await;
}

#[tokio::test]
async fn chunk_loaded_homes_have_one_ticket_and_ignore_foot_and_straw() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let mut props = WhiteBedLikeProperties::default(&Block::BLUE_BED);
    props.part = BedPart::Head;
    chunk.set_block_absolute_y(8, 64, 5, props.to_state_id(&Block::BLUE_BED));
    chunk.set_block_absolute_y(9, 64, 5, props.to_state_id(&Block::STRAW_BED));
    props.part = BedPart::Foot;
    chunk.set_block_absolute_y(10, 64, 5, props.to_state_id(&Block::BLUE_BED));
    let home = BlockPos::new(8, 64, 5);
    assert_eq!(
        fixture.world.available_homes(BlockPos::new(8, 64, 4)),
        vec![home]
    );
    let owner = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 4.5));
    let owner: Arc<dyn EntityBase> = owner;
    assert!(fixture.world.claim_home(home, Arc::downgrade(&owner)));
    assert!(fixture.world.available_homes(home).is_empty());
    fixture
        .world
        .release_home(home, owner.get_entity().entity_uuid);
    assert_eq!(fixture.world.available_homes(home), vec![home]);
    fixture.finish().await;
}

#[tokio::test]
async fn villager_walks_out_of_one_door_room_and_closes_door() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    for x in 3..=7 {
        for z in 3..=7 {
            terrain.set_block_state(x, 66, z, Block::STONE.default_state);
            if x == 3 || x == 7 || z == 3 || z == 7 {
                for y in 64..66 {
                    terrain.set_block_state(x, y, z, Block::STONE.default_state);
                }
            }
        }
    }
    let mut props = OakDoorLikeProperties::default(&Block::OAK_DOOR);
    props.facing = HorizontalFacing::East;
    props.half = DoubleBlockHalf::Lower;
    terrain.set_block_state(7, 64, 5, props.to_state_id(&Block::OAK_DOOR).to_state());
    props.half = DoubleBlockHalf::Upper;
    terrain.set_block_state(7, 65, 5, props.to_state_id(&Block::OAK_DOOR).to_state());
    publish(&world, terrain);
    let villager = spawn_villager(&world, Vector3::new(5.5, 64.0, 5.5));
    *villager.mob_entity.goals_selector.lock().unwrap() = GoalSelector::default();
    villager
        .mob_entity
        .add_goal(0, InteractWithDoorGoal::default());
    let destination = Vector3::new(10.5, 64.0, 5.5);
    let started = {
        let mut navigation = villager.mob_entity.navigator.lock().unwrap();
        let path = navigation
            .create_path(&villager.mob_entity, destination, 0)
            .unwrap();
        assert!(path.can_reach());
        navigation.move_to_path(Some(path), 0.5, &villager.mob_entity.living_entity)
    };
    assert!(started);
    let door = BlockPos::new(7, 64, 5);
    let mut opened = false;
    for _ in 0..240 {
        villager.tick(villager.as_ref(), &server);
        opened |= DoorBlock::is_open(&world, &door);
        if villager.get_entity().pos.load().x > 9.0 && !DoorBlock::is_open(&world, &door) {
            break;
        }
    }
    assert!(opened, "door never opened");
    assert!(
        villager.get_entity().pos.load().x > 9.0,
        "villager stayed at {:?}",
        villager.get_entity().pos.load()
    );
    assert!(!DoorBlock::is_open(&world, &door));
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn open_door_node_types_and_closed_door_permissions() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    let context = PathfindingContext::new(pos.0, fixture.world.clone());
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    let zombie = MobEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(3.5, 64.0, 5.5),
        &EntityType::ZOMBIE,
    ));
    zombie
        .living_entity
        .entity
        .on_ground
        .store(true, Ordering::Relaxed);
    for (block, closed) in [
        (&Block::OAK_DOOR, PathType::DoorWoodClosed),
        (&Block::IRON_DOOR, PathType::DoorIronClosed),
    ] {
        let mut props = OakDoorLikeProperties::default(block);
        for open in [true, false] {
            props.open = open;
            props.half = DoubleBlockHalf::Lower;
            chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(block));
            props.half = DoubleBlockHalf::Upper;
            chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(block));
            assert_eq!(
                context.compute_path_type_from_state(pos.0),
                if open { PathType::DoorOpen } else { closed }
            );
        }
    }
    // A solid wall leaves the door as the only route. Zombies and iron-door villagers cannot use it.
    for z in 0..16 {
        for y in 64..67 {
            chunk.set_block_absolute_y(5, y, z, Block::STONE.default_state.id);
        }
    }
    for block in [&Block::OAK_DOOR, &Block::IRON_DOOR] {
        let mut props = OakDoorLikeProperties::default(block);
        props.half = DoubleBlockHalf::Lower;
        chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(block));
        props.half = DoubleBlockHalf::Upper;
        chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(block));
        let target = Vector3::new(7.5, 64.0, 5.5);
        let can_reach = villager
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .create_path(&villager.mob_entity, target, 0)
            .is_some_and(|path| path.can_reach());
        assert_eq!(can_reach, block == &Block::OAK_DOOR);
        assert!(
            !zombie
                .navigator
                .lock()
                .unwrap()
                .create_path(&zombie, target, 0)
                .is_some_and(|path| path.can_reach())
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn zombie_break_door_flag_updates_navigation() {
    use crate::entity::mob::zombie::zombie::ZombieEntity;
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    for z in 0..16 {
        for y in 64..67 {
            chunk.set_block_absolute_y(5, y, z, Block::STONE.default_state.id);
        }
    }
    let mut props = OakDoorLikeProperties::default(&Block::OAK_DOOR);
    props.half = DoubleBlockHalf::Lower;
    chunk.set_block_absolute_y(5, 64, 5, props.to_state_id(&Block::OAK_DOOR));
    props.half = DoubleBlockHalf::Upper;
    chunk.set_block_absolute_y(5, 65, 5, props.to_state_id(&Block::OAK_DOOR));
    let zombie = ZombieEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(3.5, 64.0, 5.5),
        &EntityType::ZOMBIE,
    ));
    zombie.get_entity().on_ground.store(true, Ordering::Relaxed);
    for enabled in [false, true, false] {
        zombie.set_can_break_doors(enabled);
        let mob = zombie.get_mob_entity();
        let can_reach = mob
            .navigator
            .lock()
            .unwrap()
            .create_path(mob, Vector3::new(7.5, 64.0, 5.5), 0)
            .is_some_and(|path| path.can_reach());
        assert_eq!(can_reach, enabled);
    }
    fixture.finish().await;
}

#[tokio::test]
async fn sleeping_villager_nbt_restores_occupied_home_ticket() {
    use pumpkin_nbt::{Nbt, NbtCompound, deserializer::NbtReadHelperJava};
    let fixture = Fixture::new();
    publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let home = bed(&fixture.world, BlockPos::new(8, 64, 5));
    let original = spawn_villager(&fixture.world, Vector3::new(8.5, 64.0, 5.5));
    fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
    fixture.world.level_time.lock().unwrap().world_age = 1_000;
    prime_acquisition(&original);
    original.villager_mob_tick();
    assert!(original.get_entity().pose.load() == EntityPose::Sleeping);
    let mut saved = NbtCompound::new();
    EntityBase::write_nbt(original.as_ref(), &mut saved);
    assert_eq!(
        saved.get_int_array("sleeping_pos"),
        Some([9, 64, 5].as_slice())
    );
    let bytes = Nbt::new(String::new(), saved).write();
    let decoded = Nbt::read(&mut NbtReadHelperJava::new(std::io::Cursor::new(
        bytes.as_ref(),
    )))
    .unwrap();
    // Chunk unloading discards the live weak owner; the occupied block remains saved.
    fixture
        .world
        .release_home(home, original.get_entity().entity_uuid);
    let loaded = VillagerEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::default(),
        &EntityType::VILLAGER,
    ));
    EntityBase::read_nbt_non_mut(loaded.as_ref(), &decoded.root_tag);
    assert!(loaded.get_entity().pose.load() == EntityPose::Sleeping);
    loaded.update_home();
    assert_eq!(loaded.get_home_pos(), Some(home));
    assert!(loaded.get_entity().pose.load() == EntityPose::Sleeping);
    let rival = spawn_villager(&fixture.world, Vector3::new(7.5, 64.0, 5.5));
    rival.update_home();
    assert_eq!(rival.get_home_pos(), None);
    fixture.world.level_time.lock().unwrap().time_of_day = 10;
    loaded.rest_tick(2_000);
    assert!(loaded.get_entity().pose.load() == EntityPose::Standing);
    assert!(
        !WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&home)).occupied
    );
    fixture.finish().await;
}

pub(super) fn prime_acquisition(villager: &VillagerEntity) {
    let world = villager.get_entity().world.load_full();
    let now = world.get_world_age();
    world.level_time.lock().unwrap().world_age = now - 40;
    villager.update_home();
    world.level_time.lock().unwrap().world_age = now;
}
