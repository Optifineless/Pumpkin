use super::block_entity_round5_support::{GatedSignCapture, wait_for_crossbeam_capture};
use super::*;
use crate::block::entities::sign::{DyeColor, SignBlockEntity, SignEntityRef};
use pumpkin_util::math::position::BlockPos;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const SIGN_POSITIONS: [BlockPos; 2] = [BlockPos::new(8, 64, 8), BlockPos::new(9, 64, 8)];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_sign_setters_after_final_capture_reject_mutation_before_reload() {
    run(async {
        let scenario = Scenario::new().await;
        scenario.join(Uuid::new_v4(), "RejectNativeSign").await;
        let world = scenario.world(&Dimension::OVERWORLD);
        let (captured, wait_for_capture) = crossbeam::channel::bounded(1);
        let (resume_capture, resume_writer) = crossbeam::channel::bounded(1);
        let mut signs = Vec::new();
        let mut captures = Vec::new();
        for (index, position) in SIGN_POSITIONS.into_iter().enumerate() {
            world.set_block_state(
                &position,
                Block::OAK_SIGN.default_state.id,
                BlockFlags::FORCE_STATE,
            );
            let sign = Arc::new(SignBlockEntity::empty(position));
            let capture = Arc::new(GatedSignCapture {
                sign: sign.clone(),
                capture_id: index,
                captured: captured.clone(),
                resume: resume_writer.clone(),
                pause: AtomicBool::new(false),
            });
            world.add_block_entity(capture.clone());
            sign.front_text
                .set_message(0, format!("old {index}").into(), None);
            assert_eq!(
                sign.front_text.get_message(0, false).as_ref(),
                format!("old {index}")
            );
            signs.push(sign);
            captures.push(capture);
        }
        for capture in captures {
            capture.pause.store(true, Ordering::Release);
        }
        let unload_world = world.clone();
        let unload = std::thread::spawn(move || {
            unload_world.unload_block_entities(SIGN_POSITIONS[0].chunk_position())
        });
        let first =
            wait_for_crossbeam_capture(&wait_for_capture, "first native sign capture").await;
        resume_capture.send(()).unwrap();
        let second =
            wait_for_crossbeam_capture(&wait_for_capture, "second native sign capture").await;
        assert_ne!(first, second);

        let sign = &signs[first];
        sign.front_text.set_message(0, "native edit".into(), None);
        sign.front_text
            .clone()
            .set_message(0, "native clone edit".into(), None);
        sign.front_text
            .update_messages(["native batch edit", "", "", ""], false);
        sign.front_text.set_color(DyeColor::Red);
        sign.front_text.set_has_glowing_text(true);
        SignEntityRef::Sign(sign).set_waxed(true);
        let entity = world.get_block_entity(&SIGN_POSITIONS[first]).unwrap();
        world.update_block_entity(&entity);
        let edited = sign.front_text.get_message(0, false);
        resume_capture.send(()).unwrap();
        assert!(unload.join().unwrap());
        assert_eq!(
            edited.as_ref(),
            format!("old {first}"),
            "native setter changed sign text after final capture"
        );
        assert_eq!(sign.front_text.get_color(), DyeColor::Black);
        assert!(!sign.front_text.has_glowing_text());
        assert!(!sign.is_waxed.load(Ordering::Relaxed));

        let scenario = scenario.restart().await;
        let player = scenario.join(Uuid::new_v4(), "ReloadNativeSign").await;
        let entity = player
            .player
            .world()
            .get_block_entity(&SIGN_POSITIONS[first])
            .unwrap();
        let sign = entity.as_any().downcast_ref::<SignBlockEntity>().unwrap();
        assert_eq!(
            sign.front_text.get_message(0, false).as_ref(),
            format!("old {first}")
        );
        scenario.stop().await;
    })
    .await;
}
