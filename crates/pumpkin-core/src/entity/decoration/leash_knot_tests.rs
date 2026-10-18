use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    net::{ClientPlatform, java::combat_test_support::TestPlayer},
};
use pumpkin_data::{Block, BlockDirection, entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SInteract, SUseItemOn},
};
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::{Hand, math::vector3::Vector3};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

fn right_click(target_id: i32) -> SInteract {
    SInteract {
        entity_id: VarInt(target_id),
        r#type: VarInt(0),
        target_position: None,
        hand: Some(VarInt(0)),
        sneaking: false,
    }
}

fn place_player_near_knot(player: &TestPlayer) {
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::EMPTY.clone());
}

fn load_test_chunk(world: &World) {
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(0, 0), Arc::new(ChunkData::empty(0, 0)));
}

fn holder_entity_id(entity: &Entity) -> Option<i32> {
    let holder = entity
        .leashed_to
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    holder.map(|holder| holder.get_entity().entity_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn right_clicking_knot_returns_leash_and_removes_empty_knot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    place_player_near_knot(&player);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    load_test_chunk(&world);
    world.set_block_state(
        &BlockPos::new(0, 64, 0),
        Block::OAK_FENCE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let knot = LeashKnotEntity::create_knot(&world, BlockPos::new(0, 64, 0));
    cow.get_entity()
        .leash_to(knot.clone() as Arc<dyn EntityBase>);

    let client = player.player.client.clone();
    let actor = player.player.clone();
    let server = fixture.server.clone();
    let packet = right_click(knot.get_entity().entity_id);
    let (returned_tx, returned_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        if let ClientPlatform::Java(client) = client.as_ref() {
            client.handle_interact(&actor, &packet, &server);
            let _ = returned_tx.send(true);
        } else {
            let _ = returned_tx.send(false);
        }
    });

    assert!(
        matches!(returned_rx.recv_timeout(Duration::from_secs(30)), Ok(true)),
        "the Java interaction packet handler did not return within 30 seconds"
    );
    assert!(worker.join().is_ok(), "interaction worker panicked");
    assert_eq!(
        holder_entity_id(cow.get_entity()),
        Some(player.player.get_entity().entity_id)
    );
    assert!(!knot.get_entity().is_removed());
    knot.tick(knot.as_ref(), &fixture.server);
    assert!(knot.get_entity().is_removed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn right_clicking_knot_attaches_player_leashed_mob_to_existing_knot() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    place_player_near_knot(&player);
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    let knot = LeashKnotEntity::create_knot(&world, BlockPos::new(0, 64, 0));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    player.client().handle_interact(
        &player.player,
        &right_click(knot.get_entity().entity_id),
        &fixture.server,
    );

    let holder_id = holder_entity_id(cow.get_entity());
    assert_eq!(holder_id, Some(knot.get_entity().entity_id));
    assert!(!knot.get_entity().is_removed());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tying_player_leashed_mobs_to_a_fence_keeps_the_lead_stack() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    place_player_near_knot(&player);
    let pos = BlockPos::new(0, 64, 0);
    load_test_chunk(&world);
    world.set_block_state(
        &pos,
        Block::OAK_FENCE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, ItemStack::new(2, &Item::LEAD));
    let cow = fixture.mob(&EntityType::COW);
    cow.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    cow.get_entity()
        .leash_to(player.player.clone() as Arc<dyn EntityBase>);

    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: pos,
                face: VarInt(BlockDirection::Up as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.server,
        )
        .unwrap();

    assert_eq!(player.player.inventory().held_item().item_count, 2);
    let knot = LeashKnotEntity::get_knot(&world, pos).expect("fence tie creates a knot");
    assert_eq!(
        holder_entity_id(cow.get_entity()),
        Some(knot.get_entity().entity_id)
    );
    fixture.server.shutdown().await;
}
