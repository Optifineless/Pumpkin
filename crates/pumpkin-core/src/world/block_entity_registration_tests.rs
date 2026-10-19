use crate::{
    block::entities::{BlockEntity, sign::SignBlockEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, server::packet::PacketSentEvent},
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::Block;
use pumpkin_protocol::{ServerPacket, java::client::play::CBlockEntityData, ser::NetworkReadExt};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

const SIGN: BlockPos = BlockPos::new(8, 64, 8);

struct ReplaceDuringPacket {
    world: Weak<World>,
    replacement: Arc<SignBlockEntity>,
    called: AtomicBool,
}

impl EventHandler<PacketSentEvent> for ReplaceDuringPacket {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id == pumpkin_data::packet::clientbound::play::BLOCK_ENTITY_DATA.0
            && !self.called.swap(true, Ordering::Relaxed)
        {
            self.world
                .upgrade()
                .unwrap()
                .add_block_entity(self.replacement.clone());
        }
        Box::pin(async {})
    }
}

fn sign(label: &str) -> Arc<SignBlockEntity> {
    Arc::new(SignBlockEntity::new(
        SIGN,
        true,
        std::array::from_fn(|_| label.into()),
    ))
}

fn observed_text(player: &mut TestPlayer) -> Vec<String> {
    player
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if reader.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::BLOCK_ENTITY_DATA
                    .to_id(pumpkin_data::packet::CURRENT_MC_VERSION)
            {
                return None;
            }
            let packet =
                CBlockEntityData::read(&mut reader, &pumpkin_data::packet::CURRENT_MC_VERSION)
                    .unwrap();
            assert_eq!(packet.location, SIGN);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(
                &mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut std::io::Cursor::new(
                    packet.nbt_data.as_ref(),
                )),
            )
            .unwrap();
            Some(
                SignBlockEntity::from_nbt(&nbt.root_tag, SIGN)
                    .front_text
                    .get_message(0, false)
                    .into_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_postpublication_replacement_wins_client_server_and_saved_image() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let mut player = TestPlayer::new(&world);
    player.client().set_player(player.player.clone());
    world.set_block_state(
        &SIGN,
        Block::OAK_SIGN.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let old = sign("older registration");
    let new = sign("newer registration");
    player.take_packets();
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let weak = Arc::downgrade(&world);
    let replacement = new.clone();
    *world
        .persistence_test_hooks
        .registration_published
        .lock()
        .unwrap() = Some(Arc::new(move || {
        if !observed.swap(true, Ordering::Relaxed) {
            // A is published and unlocked; B completes before A's delivery resumes.
            weak.upgrade()
                .unwrap()
                .add_block_entity(replacement.clone());
        }
    }));

    world.add_block_entity(old);

    assert!(called.load(Ordering::Relaxed));
    let packets = observed_text(&mut player);
    assert_eq!(
        packets.last().map(String::as_str),
        Some("newer registration"),
        "stale sign text was delivered last"
    );
    if packets.iter().any(|text| text == "older registration") {
        assert_eq!(packets, vec!["older registration", "newer registration"]);
    }
    let current = world.get_block_entity(&SIGN).unwrap();
    let expected: Arc<dyn BlockEntity> = new.clone();
    assert!(Arc::ptr_eq(&current, &expected));
    let saved = world
        .level
        .loaded_chunks
        .get(&SIGN.chunk_position())
        .unwrap()
        .pending_block_entities
        .lock()
        .unwrap()
        .get(&SIGN)
        .unwrap()
        .clone();
    let restored = crate::block::entities::block_entity_from_nbt(&saved).unwrap();
    assert_eq!(restored.chunk_data_nbt(), new.chunk_data_nbt());
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_postpublication_packet_callback_keeps_successor_last() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let mut player = TestPlayer::new(&world);
    player.client().set_player(player.player.clone());
    player
        .client()
        .version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21_4);
    world.set_block_state(
        &SIGN,
        Block::OAK_SIGN.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let old = sign("callback predecessor");
    let new = sign("callback successor");
    player.take_packets();
    let hook = Arc::new(ReplaceDuringPacket {
        world: Arc::downgrade(&world),
        replacement: new,
        called: AtomicBool::new(false),
    });
    server
        .plugin_manager
        .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);

    world.add_block_entity(old);

    assert!(hook.called.load(Ordering::Relaxed));
    assert_eq!(
        observed_text(&mut player),
        vec!["callback predecessor", "callback successor"]
    );
    world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_committed_batch_unwind_delivers_without_another_owner_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    let mut player = TestPlayer::new(&world);
    player.client().set_player(player.player.clone());
    world.set_block_state(
        &SIGN,
        Block::OAK_SIGN.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    player.take_packets();
    let committed = sign("committed before batch unwind");
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _effects = crate::world::guard_effects::GuardEffects::new();
        world.add_block_entity(committed.clone());
        assert!(observed_text(&mut player).is_empty());
        panic!("abandon delivery after the registration image committed");
    }));
    assert!(failed.is_err());

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let texts = observed_text(&mut player);
            if !texts.is_empty() {
                return texts;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;

    let current = world.get_block_entity(&SIGN).unwrap();
    let expected: Arc<dyn BlockEntity> = committed;
    assert!(Arc::ptr_eq(&current, &expected));
    world.level.shutdown().await.unwrap();
    assert_eq!(
        received.expect("committed registration was discarded with its delivery batch"),
        vec!["committed before batch unwind"]
    );
}
