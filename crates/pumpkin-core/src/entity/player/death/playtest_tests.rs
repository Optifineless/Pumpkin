#![expect(
    clippy::unwrap_used,
    reason = "Respawn regression fixtures must be valid"
)]

use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{
    Block,
    damage::DamageType,
    dimension::Dimension,
    item::Item,
    packet::clientbound::play::{CONTAINER_SET_CONTENT, RESPAWN},
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_death_respawn_clears_all_slots_drops_once_and_sends_full_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(4, 63, 4, Block::STONE.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    player.get_entity().set_pos(Vector3::new(4.5, 64.0, 4.5));
    player.set_respawn_point(
        Dimension::OVERWORLD,
        BlockPos::new(4, 64, 4),
        0.0,
        0.0,
        true,
    );
    player
        .screen_handler_sync_handler
        .store_player(player.clone());
    for (slot, item) in [
        (0, &Item::DIAMOND),
        (9, &Item::STONE),
        (39, &Item::IRON_HELMET),
        (40, &Item::SHIELD),
    ] {
        player.inventory.set_slot(slot, ItemStack::new(1, item));
    }
    set_temporary_menu_items(&player);
    player.on_screen_handler_opened(&player.player_screen_handler);
    player
        .living_entity
        .damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL);
    // The old menu can broadcast the drained main/equipment slots before the respawn.
    player.sync_inventory_to_client();
    fixture.take_packets();
    let packets = fixture
        .collect_packets_during(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                world.respawn_player(&player, false),
            )
            .await
            .unwrap();
        })
        .await;
    {
        let handler = player.player_screen_handler.lock().unwrap();
        assert!(
            handler
                .get_behaviour()
                .slots
                .iter()
                .all(|slot| slot.get_stack().is_empty())
        );
        assert!(
            handler
                .get_behaviour()
                .cursor_stack
                .lock()
                .unwrap()
                .is_empty()
        );
        drop(handler);
        let entities = world.entities.load_full();
        for item in [
            &Item::DIAMOND,
            &Item::STONE,
            &Item::IRON_HELMET,
            &Item::SHIELD,
            &Item::DIRT,
            &Item::IRON_SWORD,
        ] {
            let count: u32 = entities
                .iter()
                .filter_map(|entity| entity.get_item_entity())
                .map(|entity| entity.get_item_stack().lock().unwrap().clone())
                .filter(|stack| stack.item == item)
                .map(|stack| u32::from(stack.item_count))
                .sum();
            assert_eq!(
                count, 1,
                "{} was retained or dropped twice",
                item.registry_key
            );
        }
        assert_empty_inventory_after_respawn_packet(&packets);
    };
    crate::server::fixture_lifecycle::finish().await;
}

fn set_temporary_menu_items(player: &Player) {
    let handler = player.player_screen_handler.lock().unwrap();
    handler.get_behaviour().slots[1].set_stack(ItemStack::new(1, &Item::DIRT));
    *handler.get_behaviour().cursor_stack.lock().unwrap() = ItemStack::new(1, &Item::IRON_SWORD);
}

fn assert_empty_inventory_after_respawn_packet(packets: &[bytes::Bytes]) {
    let ids: Vec<_> = packets
        .iter()
        .map(|packet| packet.as_ref().get_var_int().unwrap().0)
        .collect();
    let respawn = ids.iter().position(|id| *id == RESPAWN.0).unwrap();
    let contents = packets
        .iter()
        .enumerate()
        .find(|(i, packet)| {
            *i > respawn && packet.as_ref().get_var_int().unwrap().0 == CONTAINER_SET_CONTENT.0
        })
        .unwrap()
        .1;
    let mut data = contents.as_ref();
    assert_eq!(data.get_var_int().unwrap().0, CONTAINER_SET_CONTENT.0);
    assert_eq!(data.get_var_int().unwrap().0, 0);
    let _revision = data.get_var_int().unwrap();
    assert_eq!(data.get_var_int().unwrap().0, 46);
    // ItemStack.STREAM_CODEC encodes each empty stack (including the cursor) as count zero.
    assert_eq!(data, &[0; 47]);
}
