use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
};
use crate::{
    block::blocks::doors::DoorBlock,
    entity::{
        Entity, EntityBase,
        ai::{
            goal::interact_with_door::InteractWithDoorGoal,
            pathfinder::{
                node::{Node, PathType},
                path::Path,
                pathfinding_context::PathfindingContext,
            },
        },
        mob::Mob,
        passive::villager::VillagerEntity,
    },
    server::combat_test_support,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{DoubleBlockHalf, OakDoorLikeProperties},
    entity::EntityType,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering};

fn spawn_villager(world: &Arc<crate::world::World>, pos: Vector3<f64>) -> Arc<VillagerEntity> {
    let villager = VillagerEntity::new(Entity::new(world.clone(), pos, &EntityType::VILLAGER));
    villager
        .get_entity()
        .on_ground
        .store(true, Ordering::Relaxed);
    villager
}

fn place_door(world: &Arc<crate::world::World>, block: &Block, pos: BlockPos) {
    let mut properties = OakDoorLikeProperties::default(block);
    properties.half = DoubleBlockHalf::Lower;
    world.set_block_state(&pos, properties.to_state_id(block), BlockFlags::NOTIFY_ALL);
    properties.half = DoubleBlockHalf::Upper;
    world.set_block_state(
        &pos.up(),
        properties.to_state_id(block),
        BlockFlags::NOTIFY_ALL,
    );
}

struct DoorEvents(std::sync::Mutex<Vec<(String, Option<uuid::Uuid>)>>);

impl EventHandler<GenericGameEvent> for DoorEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push((event.event_key.clone(), event.source_entity));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_path_node_opens_its_door_before_the_repeat_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let events = Arc::new(DoorEvents(std::sync::Mutex::new(Vec::new())));
    server.plugin_manager.register::<GenericGameEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    let first = BlockPos::new(6, 64, 5);
    let next = BlockPos::new(7, 64, 5);
    place_door(&world, &Block::OAK_DOOR, first);
    place_door(&world, &Block::OAK_DOOR, next);
    let villager = spawn_villager(&world, Vector3::new(5.5, 64.0, 5.5));
    let started = {
        let mut navigation = villager.mob_entity.navigator.lock().unwrap();
        navigation.move_to_path(
            Some(Path::new(
                vec![Node::new(first), Node::new(next)],
                next,
                true,
            )),
            0.5,
            &villager.mob_entity.living_entity,
        )
    };
    assert!(started);

    let mut goal = InteractWithDoorGoal::default();
    goal.tick(villager.as_ref());
    assert!(DoorBlock::is_open(&world, &first));
    let source = villager.get_entity().entity_uuid;
    goal.tick(villager.as_ref());
    assert_eq!(events.0.lock().unwrap().len(), 1);
    assert_eq!(
        *events.0.lock().unwrap(),
        vec![(
            pumpkin_data::game_event::GameEvent::BlockOpen
                .name()
                .to_owned(),
            Some(source)
        )]
    );
    villager
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .get_path_mut()
        .unwrap()
        .advance();
    goal.tick(villager.as_ref());
    assert!(DoorBlock::is_open(&world, &next));
    assert_eq!(
        *events.0.lock().unwrap(),
        vec![
            (
                pumpkin_data::game_event::GameEvent::BlockOpen
                    .name()
                    .to_owned(),
                Some(source)
            );
            2
        ]
    );
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn open_doors_are_open_nodes_and_villagers_only_path_through_wooden_doors() {
    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(5, 64, 5);
    let context = PathfindingContext::new(pos.0, fixture.world.clone());
    let villager = spawn_villager(&fixture.world, Vector3::new(3.5, 64.0, 5.5));
    for (block, closed_type) in [
        (&Block::OAK_DOOR, PathType::DoorWoodClosed),
        (&Block::IRON_DOOR, PathType::DoorIronClosed),
    ] {
        let mut properties = OakDoorLikeProperties::default(block);
        for open in [true, false] {
            properties.open = open;
            properties.half = DoubleBlockHalf::Lower;
            chunk.set_block_absolute_y(5, 64, 5, properties.to_state_id(block));
            properties.half = DoubleBlockHalf::Upper;
            chunk.set_block_absolute_y(5, 65, 5, properties.to_state_id(block));
            assert_eq!(
                context.compute_path_type_from_state(pos.0),
                if open {
                    PathType::DoorOpen
                } else {
                    closed_type
                }
            );
        }
    }

    for z in 0..16 {
        for y in 64..67 {
            chunk.set_block_absolute_y(5, y, z, Block::STONE.default_state.id);
        }
    }
    for block in [&Block::OAK_DOOR, &Block::IRON_DOOR] {
        let mut properties = OakDoorLikeProperties::default(block);
        properties.half = DoubleBlockHalf::Lower;
        chunk.set_block_absolute_y(5, 64, 5, properties.to_state_id(block));
        properties.half = DoubleBlockHalf::Upper;
        chunk.set_block_absolute_y(5, 65, 5, properties.to_state_id(block));
        let can_reach = villager
            .mob_entity
            .navigator
            .lock()
            .unwrap()
            .create_path(&villager.mob_entity, Vector3::new(7.5, 64.0, 5.5), 0)
            .is_some_and(|path| path.can_reach());
        assert_eq!(can_reach, block == &Block::OAK_DOOR);
    }
    fixture.finish().await;
}

#[tokio::test]
async fn zombie_door_breaking_flag_updates_navigation() {
    use crate::entity::mob::zombie::zombie::ZombieEntity;

    let fixture = Fixture::new();
    let chunk = publish(&fixture.world, proto(&Biome::PLAINS, &Block::STONE));
    for z in 0..16 {
        for y in 64..67 {
            chunk.set_block_absolute_y(5, y, z, Block::STONE.default_state.id);
        }
    }
    let mut properties = OakDoorLikeProperties::default(&Block::OAK_DOOR);
    properties.half = DoubleBlockHalf::Lower;
    chunk.set_block_absolute_y(5, 64, 5, properties.to_state_id(&Block::OAK_DOOR));
    properties.half = DoubleBlockHalf::Upper;
    chunk.set_block_absolute_y(5, 65, 5, properties.to_state_id(&Block::OAK_DOOR));
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
