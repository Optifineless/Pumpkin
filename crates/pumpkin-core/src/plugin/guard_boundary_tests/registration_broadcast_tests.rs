use super::Fixture;
use crate::block::entities::{BlockEntity, sign::SignBlockEntity};
use pumpkin_data::{Block, packet::CURRENT_MC_VERSION};
use pumpkin_protocol::{ServerPacket, java::client::play::CBlockEntityData, ser::NetworkReadExt};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::{sync::Arc, time::Duration};

const POSITION: BlockPos = BlockPos::new(2, 64, 2);
const BOUND: Duration = Duration::from_secs(5);

fn sign(label: &str) -> Arc<dyn BlockEntity> {
    Arc::new(SignBlockEntity::new(
        POSITION,
        true,
        std::array::from_fn(|_| label.into()),
    ))
}

fn observed_text(packets: Vec<bytes::Bytes>) -> Vec<String> {
    packets
        .into_iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if reader.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::BLOCK_ENTITY_DATA.0
            {
                return None;
            }
            let packet = CBlockEntityData::read(&mut reader, &CURRENT_MC_VERSION).unwrap();
            assert_eq!(packet.location, POSITION);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(
                &mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut std::io::Cursor::new(
                    packet.nbt_data.as_ref(),
                )),
            )
            .unwrap();
            Some(
                SignBlockEntity::from_nbt(&nbt.root_tag, POSITION)
                    .front_text
                    .get_message(0, false)
                    .into_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_registration_does_not_broadcast_superseded_sign_text() {
    let mut fixture = Fixture::new();
    fixture.world.set_block_state(
        &POSITION,
        Block::OAK_SIGN.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    fixture.player.take_packets();
    let first = sign("first");
    let current = sign("current");
    let (registered, wait_for_registration) = crossbeam::channel::bounded(1);
    let (resume, wait_for_resume) = crossbeam::channel::bounded(1);
    *fixture
        .world
        .persistence_test_hooks
        .registration
        .lock()
        .unwrap() = Some(Arc::new(move || {
        registered.send(()).unwrap();
        wait_for_resume.recv_timeout(BOUND).unwrap();
    }));
    let world = fixture.world.clone();
    let runtime = tokio::runtime::Handle::current();
    let (finished, wait_for_finish) = crossbeam::channel::bounded(1);
    drop(std::thread::spawn(move || {
        let _runtime = runtime.enter();
        world.add_block_entity(first);
        finished.send(()).unwrap();
    }));
    wait_for_registration.recv_timeout(BOUND).unwrap();
    fixture
        .world
        .persistence_test_hooks
        .registration
        .lock()
        .unwrap()
        .take();
    fixture.world.add_block_entity(current.clone());
    resume.send(()).unwrap();
    wait_for_finish.recv_timeout(BOUND).unwrap();

    let packets = observed_text(fixture.player.take_packets());
    assert!(fixture.world.is_live_block_entity(&current));
    assert_eq!(
        packets,
        vec!["current"],
        "rejected registration sent stale sign text after the current update"
    );
    tokio::time::timeout(BOUND, fixture.close()).await.unwrap();
}
