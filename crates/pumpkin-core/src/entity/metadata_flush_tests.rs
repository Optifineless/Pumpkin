use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::tracked_data::player::PLAYER_MODE_CUSTOMISATION;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::client::play::MetadataSerializer,
    ser::{NetworkReadExt, WritingError},
};
use pumpkin_util::version::JavaMinecraftVersion;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

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

#[tokio::test]
async fn concurrent_metadata_flush_and_refresh_enqueue_revisions_in_order() {
    for (first_refresh, second_refresh) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let directory = tempfile::tempdir().unwrap();
        let server = server(directory.path());
        let world = world(&server, directory.path());
        let mut viewer = TestPlayer::new(&world);
        let player = viewer.player.clone();
        let entity = player.get_entity();
        entity.set_synced_data(PLAYER_MODE_CUSTOMISATION, 1u8);
        viewer.take_packets();
        let (paused, arrived) = mpsc::channel();
        let (resume, proceed) = mpsc::channel();
        entity.metadata_flush.lock().unwrap().before_enqueue = Some(Box::new(move || {
            paused.send(()).unwrap();
            proceed.recv().unwrap();
        }));
        let first = viewer.player.clone();
        let sender = std::thread::spawn(move || {
            if first_refresh {
                first.get_entity().refresh_synced_data();
            } else {
                first.get_entity().send_dirty_entity_data();
            }
        });
        arrived.recv().unwrap();
        entity.set_synced_data(PLAYER_MODE_CUSTOMISATION, 2u8);
        if second_refresh {
            entity.refresh_synced_data();
        } else {
            entity.send_dirty_entity_data();
        }
        resume.send(()).unwrap();
        sender.join().unwrap();
        let id = viewer.player.entity_id();
        assert_eq!(
            skin_metadata(&mut viewer, id),
            [1, 2],
            "overlapping sends left stale client metadata"
        );
        assert!(!viewer.player.get_entity().synched_data.is_dirty());
        world.level.shutdown().await.unwrap();
    }
}

#[derive(Clone)]
struct UnwindingMetadata {
    value: u8,
    unwind_once: Arc<AtomicBool>,
    reentrant_refresh: Option<Weak<Player>>,
}

impl MetadataSerializer for UnwindingMetadata {
    fn write_metadata(
        &self,
        writer: &mut impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        let unwind = self.unwind_once.swap(false, Ordering::AcqRel);
        if unwind && let Some(player) = &self.reentrant_refresh {
            player.upgrade().unwrap().get_entity().refresh_synced_data();
        }
        assert!(!unwind, "one-shot metadata serializer unwind");
        self.value.write_metadata(writer, version)
    }
}

fn assert_metadata_flush_idle(player: &crate::entity::player::Player) {
    let state = player.get_entity().metadata_flush.lock().unwrap();
    assert!(!state.running && !state.refresh_pending && !state.dirty_pending);
}

#[tokio::test]
async fn ordinary_tracker_retries_a_clean_queued_refresh_after_sender_unwind() {
    for reentrant in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let server = server(directory.path());
        let world = world(&server, directory.path());
        let mut viewer = TestPlayer::new(&world);
        let entity = viewer.player.get_entity();
        let unwind_once = Arc::new(AtomicBool::new(false));
        entity.set_synced_data(
            PLAYER_MODE_CUSTOMISATION,
            UnwindingMetadata {
                value: 0x7f,
                unwind_once: unwind_once.clone(),
                reentrant_refresh: reentrant.then(|| Arc::downgrade(&viewer.player)),
            },
        );
        world.entity_tracker.update_all(&world);
        let id = viewer.player.entity_id();
        assert_eq!(skin_metadata(&mut viewer, id), [0x7f]);
        assert!(!viewer.player.get_entity().synched_data.is_dirty());

        let (paused, arrived) = mpsc::channel();
        let (resume, proceed) = mpsc::channel();
        if !reentrant {
            viewer
                .player
                .get_entity()
                .metadata_flush
                .lock()
                .unwrap()
                .before_enqueue = Some(Box::new(move || {
                paused.send(()).unwrap();
                proceed.recv().unwrap();
            }));
        }
        unwind_once.store(true, Ordering::Release);
        let first = viewer.player.clone();
        let sender = std::thread::spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                first.get_entity().refresh_synced_data();
            }))
        });
        if !reentrant {
            arrived.recv().unwrap();
            let entity = viewer.player.get_entity();
            assert!(!entity.synched_data.is_dirty());
            entity.refresh_synced_data();
            assert!(entity.metadata_flush.lock().unwrap().refresh_pending);
            resume.send(()).unwrap();
        }
        assert!(sender.join().unwrap().is_err());
        assert!(
            viewer
                .player
                .get_entity()
                .metadata_flush
                .lock()
                .unwrap()
                .refresh_pending
        );
        assert!(skin_metadata(&mut viewer, id).is_empty());

        // No setter or explicit refresh: the normal tracker tick must admit the pending request.
        world.entity_tracker.update_all(&world);
        assert_eq!(
            skin_metadata(&mut viewer, id),
            [0x7f],
            "ordinary tracker lost the queued clean refresh after sender unwind"
        );
        assert!(!viewer.player.get_entity().synched_data.is_dirty());
        assert_metadata_flush_idle(&viewer.player);
        world.level.shutdown().await.unwrap();
    }
}
