use super::{regression_tests::tracked_value, tests::hook, *};
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::fish::{PlayerFishEvent, PlayerFishState},
    },
    server::combat_test_support::{server, world},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::position::BlockPos;
use std::sync::atomic::AtomicUsize;

struct CancelFish {
    state: PlayerFishState,
    calls: AtomicUsize,
}

impl EventHandler<PlayerFishEvent> for CancelFish {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerFishEvent,
    ) -> BoxFuture<'a, ()> {
        if event.state == self.state {
            self.calls.fetch_add(1, Relaxed);
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

fn cancel(server: &Arc<Server>, state: PlayerFishState) -> Arc<CancelFish> {
    let handler = Arc::new(CancelFish {
        state,
        calls: AtomicUsize::new(0),
    });
    server.plugin_manager.register::<PlayerFishEvent, _>(
        handler.clone(),
        EventPriority::Normal,
        true,
    );
    handler
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_cancelled_failed_attempt_preserves_nibble_timers_and_biting() {
    // The fork event gates FishingHook.catchingFish's final nibble/expiry mutations.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let bobber = hook(&world, &owner.player);
    let handler = cancel(&server, PlayerFishState::FailedAttempt);
    bobber.bite_countdown.store(1, Relaxed);
    bobber.wait_countdown.store(88, Relaxed);
    bobber.hook_countdown.store(9, Relaxed);
    bobber
        .entity
        .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, true);
    bobber.entity.synched_data.clear_dirty();
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut Vector3::default());
    assert_eq!(handler.calls.load(Relaxed), 1);
    assert_eq!(bobber.bite_countdown.load(Relaxed), 1);
    assert_eq!(bobber.wait_countdown.load(Relaxed), 88);
    assert_eq!(bobber.hook_countdown.load(Relaxed), 9);
    assert!(!bobber.entity.synched_data.is_dirty());
    // A no-op write proves DATA_BITING stayed true, even if dirty flags were cleared.
    bobber
        .entity
        .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, true);
    assert!(!bobber.entity.synched_data.is_dirty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_cancelled_bite_skips_nibble_metadata_kick_sound_and_particles() {
    // The fork event gates FishingHook.catchingFish's bite effects.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut owner = TestPlayer::new(&world);
    publish(&world, proto(&Biome::PLAINS, &Block::AIR));
    let bobber = hook(&world, &owner.player);
    let handler = cancel(&server, PlayerFishState::Bite);
    bobber.hook_countdown.store(1, Relaxed);
    bobber
        .entity
        .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, false);
    bobber.entity.synched_data.clear_dirty();
    owner.take_packets();
    let mut velocity = Vector3::new(0.1, 0.2, 0.3);
    bobber.catching_fish(&world, &BlockPos::new(0, 64, 0), &mut velocity);
    assert_eq!(handler.calls.load(Relaxed), 1);
    assert_eq!(bobber.bite_countdown.load(Relaxed), 0);
    assert_eq!(velocity, Vector3::new(0.1, 0.2, 0.3));
    assert!(!bobber.entity.synched_data.is_dirty());
    bobber
        .entity
        .set_synced_data(tracked_data::fishing_bobber::DATA_BITING, true);
    assert_eq!(
        tracked_value(&bobber.entity, tracked_data::fishing_bobber::DATA_BITING),
        [1]
    );
    assert!(owner.take_packets().is_empty());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_cancelled_caught_entity_skips_pull_status_durability_and_removal() {
    // The fork event gates FishingHook.retrieve/pullEntity before any effects.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut owner = TestPlayer::new(&world);
    let bobber = Arc::new(hook(&world, &owner.player));
    world.spawn_entity(bobber.clone());
    owner
        .player
        .fishing_bobber
        .store(bobber.entity.entity_id, Relaxed);
    let target = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        owner.player.position().add_raw(5.0, 0.0, 0.0),
        &EntityType::COW,
    )));
    target.entity.velocity.store(Vector3::new(0.1, 0.2, 0.3));
    world.add_entity_silent(target.clone());
    bobber.entity.set_pos(target.entity.pos.load());
    bobber.set_hooked_entity(Some(target.entity.entity_id));
    let handler = cancel(&server, PlayerFishState::CaughtEntity);
    owner.take_packets();
    owner.client().handle_use_item(
        &owner.player,
        &pumpkin_protocol::java::server::play::SUseItem {
            hand: pumpkin_protocol::VarInt(0),
            sequence: pumpkin_protocol::VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    assert_eq!(handler.calls.load(Relaxed), 1);
    assert_eq!(target.entity.velocity.load(), Vector3::new(0.1, 0.2, 0.3));
    assert!(!bobber.entity.is_removed());
    assert_eq!(
        owner.player.fishing_bobber.load(Relaxed),
        bobber.entity.entity_id
    );
    assert_eq!(owner.player.inventory().held_item().get_damage(), 0);
    assert!(owner.take_packets().iter().all(|bytes| {
        let id = bytes.as_ref().get_var_int().unwrap().0;
        id != pumpkin_data::packet::clientbound::play::ENTITY_EVENT.0
            && id != pumpkin_data::packet::clientbound::play::SOUND.0
    }));
    crate::server::fixture_lifecycle::finish().await;
}
