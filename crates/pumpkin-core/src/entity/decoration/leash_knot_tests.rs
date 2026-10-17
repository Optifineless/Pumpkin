use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    net::{ClientPlatform, java::combat_test_support::TestPlayer},
};
use pumpkin_data::{entity::EntityType, item_stack::ItemStack};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SInteract};
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::{sync::mpsc, time::Duration};

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
