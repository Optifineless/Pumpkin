use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::tracked_data::player::PLAYER_MODE_CUSTOMISATION;
use pumpkin_protocol::{codec::var_int::VarInt, ser::NetworkReadExt};
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::Arc;

fn skin_metadata(player: &mut TestPlayer, entity_id: i32) -> Vec<u8> {
    player
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if VarInt::decode(&mut reader).unwrap().0
                != pumpkin_data::packet::clientbound::play::SET_ENTITY_DATA.0
            {
                return None;
            }
            if reader.get_var_int().unwrap().0 != entity_id {
                return None;
            }
            // Player model customisation is index 16, byte serializer 0 in the extracted 26.3 data.
            let index = u8::from(PLAYER_MODE_CUSTOMISATION.id);
            reader
                .windows(3)
                .find(|entry| entry[0] == index && entry[1] == 0)
                .map(|entry| entry[2])
        })
        .collect()
}

#[tokio::test]
async fn tracker_flush_keeps_a_revision_written_between_packing_and_clearing() {
    for refresh in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let server = server(directory.path());
        let world = world(&server, directory.path());
        let mut first = TestPlayer::new(&world);
        let mut legacy = TestPlayer::new(&world);
        legacy
            .client()
            .version
            .store(JavaMinecraftVersion::V_1_20_5);
        world
            .players
            .store(Arc::new(vec![first.player.clone(), legacy.player.clone()]));
        legacy
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(first.player.get_entity().chunk_pos.load());
        world
            .entity_tracker
            .update_entity_position(first.player.as_ref(), &world);
        assert!(
            world
                .entity_tracker
                .get_tracked_entity(first.player.entity_id())
                .unwrap()
                .seen_by
                .contains(&legacy.player.gameprofile.id)
        );
        world.entity_tracker.update_all(&world);
        let entity = first.player.get_entity();
        entity.set_synced_data(PLAYER_MODE_CUSTOMISATION, 1u8);
        let player = Arc::downgrade(&first.player);
        *entity.synched_data.before_dirty_clear.lock().unwrap() = Some(Box::new(move || {
            player
                .upgrade()
                .unwrap()
                .get_entity()
                .set_synced_data(PLAYER_MODE_CUSTOMISATION, 2u8);
        }));
        first.take_packets();
        legacy.take_packets();
        if refresh {
            first.player.get_entity().refresh_synced_data();
        } else {
            world.entity_tracker.update_all(&world);
        }
        let id = first.player.entity_id();
        assert_eq!(skin_metadata(&mut first, id), [1]);
        assert_eq!(skin_metadata(&mut legacy, id), [1]);
        assert!(first.player.get_entity().synched_data.is_dirty());
        world.entity_tracker.update_all(&world);
        assert_eq!(skin_metadata(&mut first, id), [2]);
        assert_eq!(skin_metadata(&mut legacy, id), [2]);
        assert!(!first.player.get_entity().synched_data.is_dirty());
        world.level.shutdown().await.unwrap();
    }
}
