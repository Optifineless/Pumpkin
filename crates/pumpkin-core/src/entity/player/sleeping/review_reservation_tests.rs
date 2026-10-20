use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::{
    Block, block_properties::WhiteBedLikeProperties, damage::DamageType, dimension::Dimension,
    entity::EntityPose, item::Item,
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{Action, SClientCommand, SPlayerAction, SPlayerCommand, SUseItemOn},
};
use pumpkin_util::{
    GameMode, PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};

use super::{
    Callback, EntityBase, EventPriority, FOOT, HEAD, TestPlayer, position_rules::Scenario,
};
use crate::{
    plugin::api::events::block::block_break::BlockBreakEvent,
    server::combat_test_support::wait_for_tasks,
};

#[derive(Clone, Copy)]
enum Wake {
    Native,
    JavaLeave,
    LethalDamage,
}

fn assert_released(scenario: &Scenario) {
    let player = &scenario.client.player;
    assert_eq!(player.sleeping_bed_pos.load(), None);
    assert_eq!(player.sleeping_since.load(), None);
    assert!(!scenario.world.should_skip_night());
    let expected_pose = if player.living_entity.is_alive() {
        EntityPose::Standing
    } else {
        EntityPose::Dying
    };
    assert!(player.get_entity().pose.load() == expected_pose);
    for position in [FOOT, HEAD] {
        assert_eq!(scenario.world.get_block(&position), &Block::WHITE_BED);
        assert!(
            !WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&position))
                .occupied,
            "the live break reservation must allow release at {position:?}"
        );
    }
}

fn on_break_wake(
    scenario: &Scenario,
    position: BlockPos,
    wake: Wake,
    cancel: bool,
) -> Arc<AtomicUsize> {
    let sleeper = Arc::downgrade(&scenario.client.player);
    let server = Arc::downgrade(&scenario.server);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    scenario
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(
            Arc::new(Callback(move |event: &mut BlockBreakEvent| {
                if event.block != &Block::WHITE_BED || event.block_position != position {
                    return;
                }
                seen.fetch_add(1, Relaxed);
                let player = sleeper.upgrade().unwrap();
                assert_eq!(player.sleeping_bed_pos.load(), Some(HEAD));
                match wake {
                    Wake::Native => player.wake_up(),
                    Wake::JavaLeave => {
                        let client = player.client.java().unwrap();
                        client.handle_player_command(
                            &player,
                            &SPlayerCommand {
                                entity_id: player.entity_id().into(),
                                action: Action::LeaveBed,
                                jump_boost: VarInt(0),
                            },
                            &server.upgrade().unwrap(),
                        );
                    }
                    Wake::LethalDamage => {
                        assert!(player.damage(player.as_ref(), f32::MAX, DamageType::GENERIC_KILL));
                        assert_eq!(player.living_entity.health.load(), 0.0);
                    }
                }
                assert_eq!(
                    player.sleeping_bed_pos.load(),
                    None,
                    "an admitted wake must finish before the cancelled break callback returns"
                );
                assert_eq!(player.sleeping_since.load(), None);
                player.sleep(HEAD);
                assert_eq!(
                    player.sleeping_bed_pos.load(),
                    None,
                    "release must preserve the outer reservation's refusal of new entry"
                );
                event.cancelled = cancel;
            })),
            EventPriority::Normal,
            true,
        );
    calls
}

fn breaker(scenario: &Scenario) -> TestPlayer {
    let breaker = TestPlayer::new(&scenario.world);
    breaker.player.permission_lvl.store(PermissionLvl::Four);
    breaker.player.gamemode.store(GameMode::Creative);
    breaker
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 6.5));
    scenario.world.players.store(Arc::new(vec![
        scenario.client.player.clone(),
        scenario.observer.player.clone(),
        breaker.player.clone(),
    ]));
    breaker
}

async fn break_callback_wake(position: BlockPos, wake: Wake, cancel: bool) {
    let mut scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    scenario
        .server
        .worlds
        .store(Arc::new(vec![scenario.world.clone()]));
    let breaker = breaker(&scenario);
    let calls = on_break_wake(&scenario, position, wake, cancel);
    breaker.client().handle_player_action(
        &breaker.player,
        &SPlayerAction {
            status: VarInt(0),
            position,
            face: 1,
            sequence: VarInt(2),
        },
        &scenario.server,
    );
    assert_eq!(calls.load(Relaxed), 1, "real bed-break callback must run");
    assert_released(&scenario);
    if matches!(wake, Wake::LethalDamage) {
        let player = scenario.client.player.clone();
        scenario.client.client().handle_client_status(
            &player,
            &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
        );
        tokio::time::timeout(
            Duration::from_secs(60),
            scenario
                .client
                .with_outgoing_writer(wait_for_tasks(&scenario.server)),
        )
        .await
        .unwrap();
        assert!(player.living_entity.health.load() > 0.0);
        assert_released(&scenario);
        player.set_client_loaded(true);
    }
    scenario
        .client
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 6.5));
    scenario.use_on(FOOT);
    assert_eq!(scenario.client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert!(scenario.client.player.get_entity().pose.load() == EntityPose::Sleeping);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_cancelled_break_native_wake() {
    for position in [HEAD, FOOT] {
        break_callback_wake(position, Wake::Native, true).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_cancelled_break_java_leave() {
    for position in [HEAD, FOOT] {
        break_callback_wake(position, Wake::JavaLeave, true).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_cancelled_break_lethal_respawn() {
    for position in [HEAD, FOOT] {
        break_callback_wake(position, Wake::LethalDamage, true).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_wake_invalidates_uncancelled_break() {
    for position in [HEAD, FOOT] {
        break_callback_wake(position, Wake::Native, false).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_unsafe_use_reservation_allows_wake() {
    let scenario = Scenario::with_native_sleep(Dimension::THE_NETHER, &Item::WHITE_BED);
    assert!(scenario.world.dimension.bed_rule.destroy_on_use);
    let user = breaker(&scenario);
    let calls = on_break_wake(&scenario, HEAD, Wake::Native, true);
    user.client()
        .handle_use_item_on(
            &user.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: FOOT,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(2),
            },
            &scenario.server,
        )
        .unwrap();
    assert_eq!(
        calls.load(Relaxed),
        1,
        "unsafe use reaches the break callback"
    );
    assert_released(&scenario);
    scenario.finish().await;
}
