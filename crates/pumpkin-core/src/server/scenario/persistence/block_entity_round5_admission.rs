use super::block_entity_round5_support::{GatedSignCapture, wait_for_crossbeam_capture};
use super::block_entity_support::*;
use super::*;
use crate::block::entities::{BlockEntity, chest::ChestBlockEntity, sign::SignBlockEntity};
use pumpkin_protocol::java::server::play::SUpdateSign;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const SECOND_CHEST: BlockPos = BlockPos::new(9, 64, 8);
const SIGN_A: BlockPos = BlockPos::new(8, 64, 8);
const SIGN_B: BlockPos = BlockPos::new(9, 64, 8);

async fn assert_reloaded_sign_text(
    scenario: Scenario,
    player_id: Uuid,
    position: BlockPos,
    expected: &str,
) {
    scenario.stop().await;
    let scenario = scenario.restart().await;
    let player = scenario.join(player_id, "ReloadLateSign").await;
    let loaded = player
        .player
        .world()
        .get_block_entity(&position)
        .expect("reloaded sign missing after unload");
    let sign = loaded.as_any().downcast_ref::<SignBlockEntity>().unwrap();
    assert_eq!(sign.front_text.get_message(0, false).as_ref(), expected);
    scenario.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unload_admission_rejects_a_new_chest_before_terrain_commit() {
    run(async {
        let scenario = Scenario::new().await;
        scenario.join(Uuid::new_v4(), "RejectLateChest").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        world.set_block_state(
            &CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let (captured, wait_for_capture) = mpsc::channel();
        let (resume_capture, resume_writer) = mpsc::channel();
        let paused = Arc::new(PausedChestCapture {
            chest: Arc::new(ChestBlockEntity::new(CHEST)),
            captured,
            resume: Mutex::new(Some(resume_writer)),
            pause: AtomicBool::new(false),
        });
        world.add_block_entity(paused.clone());
        paused.pause.store(true, Ordering::Release);

        let unload_world = world.clone();
        let unload = std::thread::spawn(move || {
            unload_world.unload_block_entities(CHEST.chunk_position());
        });
        tokio::task::spawn_blocking(move || {
            wait_for_std_barrier(&wait_for_capture, "unload chest capture");
        })
        .await
        .unwrap();

        world.set_block_state(
            &SECOND_CHEST,
            Block::CHEST.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        let late_state = world.get_block_state(&SECOND_CHEST).id;

        resume_capture.send(()).unwrap();
        unload.join().unwrap();
        assert_eq!(
            late_state,
            Block::AIR.default_state.id,
            "late chest terrain was committed after block-entity admission closed"
        );
        scenario.stop().await;

        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadNoLateChest").await;
        let world = player.player.world();
        assert_eq!(
            world.get_block_state(&SECOND_CHEST).id,
            Block::AIR.default_state.id
        );
        assert!(world.get_block_entity(&SECOND_CHEST).is_none());
        scenario.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_packet_after_unload_admission_is_rejected_before_text_changes() {
    run(async {
        let scenario = Scenario::new().await;
        let player_id = Uuid::new_v4();
        let player = scenario.join(player_id, "RejectLateSign").await;
        let world = player.player.world();
        world.set_block_state(
            &SIGN_A,
            Block::OAK_SIGN.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        world.set_block_state(
            &SIGN_B,
            Block::OAK_SIGN.default_state.id,
            BlockFlags::FORCE_STATE,
        );

        let sign_a = Arc::new(SignBlockEntity::empty(SIGN_A));
        let sign_b = Arc::new(SignBlockEntity::empty(SIGN_B));
        sign_a.front_text.set_message(0, "old A".into(), None);
        sign_b.front_text.set_message(0, "old B".into(), None);
        *sign_a
            .currently_editing_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player_id);
        *sign_b
            .currently_editing_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player_id);

        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let gate_a = Arc::new(GatedSignCapture {
            sign: sign_a.clone(),
            capture_id: 0,
            captured: captured.clone(),
            resume: resume_writer.clone(),
            pause: AtomicBool::new(false),
        });
        let gate_b = Arc::new(GatedSignCapture {
            sign: sign_b.clone(),
            capture_id: 1,
            captured,
            resume: resume_writer,
            pause: AtomicBool::new(false),
        });
        let entity_a: Arc<dyn BlockEntity> = gate_a.clone();
        let entity_b: Arc<dyn BlockEntity> = gate_b.clone();
        world.add_block_entity(entity_a);
        world.add_block_entity(entity_b);
        gate_a.pause.store(true, Ordering::Release);
        gate_b.pause.store(true, Ordering::Release);

        let unload_world = world.clone();
        let unload = std::thread::spawn(move || {
            unload_world.unload_block_entities(SIGN_A.chunk_position());
        });
        let first_id =
            wait_for_crossbeam_capture(&wait_for_capture, "first sign unload capture").await;
        resume_capture.send(()).unwrap();
        let second_id =
            wait_for_crossbeam_capture(&wait_for_capture, "second sign unload capture").await;
        assert_ne!(first_id, second_id);

        let (position, sign) = if first_id == 0 {
            (SIGN_A, &sign_a)
        } else {
            (SIGN_B, &sign_b)
        };
        let packet = SUpdateSign {
            location: position,
            is_front_text: true,
            line_1: "new text",
            line_2: "",
            line_3: "",
            line_4: "",
        };
        player.client().handle_sign_update(&player.player, &packet);
        let edited_text = sign.front_text.get_message(0, false);
        resume_capture.send(()).unwrap();
        unload.join().unwrap();
        assert_eq!(
            edited_text.as_ref(),
            if first_id == 0 { "old A" } else { "old B" },
            "sign packet changed text after unload admission closed"
        );

        assert_reloaded_sign_text(
            scenario,
            player_id,
            position,
            if first_id == 0 { "old A" } else { "old B" },
        )
        .await;
    })
    .await;
}
