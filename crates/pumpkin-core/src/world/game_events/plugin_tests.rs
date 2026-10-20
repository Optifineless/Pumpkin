use super::test_support::{DEPARTURE, Scenario, use_item};
use crate::{
    entity::EntityBase,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block,
    block_properties::{AttachFace, LeverLikeProperties},
    game_event::GameEvent,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

fn mutation(key: &'static str, expected: GameEvent) {
    let scenario = Scenario::new(true);
    scenario.rod(Some(true));
    scenario.capture(Some(Box::new(move |_, event| {
        assert_eq!(event.event_key, "item_interact_start");
        event.event_key = key.into();
        event.position = Vector3::new(3.5, 70.0, -4.5);
    })));
    scenario.use_item();
    assert!(
        scenario
            .world
            .get_entity_by_id(scenario.owner.player.fishing_bobber.load(Relaxed))
            .is_some()
    );
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.plugins[0].key, "item_interact_start");
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(
        sink.forwarded[0].event, expected,
        "dispatch must resolve the final plugin key"
    );
    assert_eq!(
        sink.forwarded[0].position,
        Vector3::new(3.5, 70.0, -4.5),
        "dispatch must use the final plugin position"
    );
    scenario.assert_source(sink.forwarded[0].source.as_ref().unwrap(), DEPARTURE);
}

fn suppressed(mode: u8) {
    let scenario = Scenario::new(true);
    scenario.rod(Some(true));
    let player = scenario.owner.player.clone();
    scenario.capture(Some(Box::new(move |_, event| match mode {
        0 => event.cancelled = true,
        1 => event.event_key = "plugin:unknown_vibration".into(),
        2 => event.position.x = f64::NAN,
        3 => event.position.z = 30_000_000.0,
        4 => player.client.java().unwrap().close(),
        5 => player.get_entity().remove(),
        6 => {
            // Tracker lookup can still find the old player; active session membership must reject it.
            let _replacement = crate::net::java::combat_test_support::TestPlayer::with_profile(
                &player.world(),
                player.gameprofile.clone(),
            );
        }
        _ => panic!("invalid suppression scenario"),
    })));
    scenario.use_item();
    // The already admitted cast is committed even when its notification is cancelled/stale.
    assert!(
        scenario
            .world
            .get_entity_by_id(scenario.owner.player.fishing_bobber.load(Relaxed))
            .is_some()
    );
    assert_eq!(
        scenario.owner.player.inventory().held_item().get_damage(),
        0
    );
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.plugins[0].key, "item_interact_start");
    assert!(
        sink.forwarded.is_empty(),
        "mode {mode}: cancelled/unknown/invalid/stale event must not forward"
    );
}

fn reentry() {
    let scenario = Scenario::new(true);
    scenario.rod(Some(true));
    let player = scenario.owner.player.clone();
    let entered = Arc::new(AtomicBool::new(false));
    scenario.capture(Some(Box::new(move |server, _| {
        if !entered.swap(true, Relaxed) {
            let hook = player
                .world()
                .get_entity_by_id(player.fishing_bobber.load(Relaxed))
                .unwrap();
            hook.get_entity().on_ground.store(true, Relaxed);
            use_item(&player, server); // A genuine nested reel, bounded to one action.
        }
    })));
    scenario.use_item();
    assert_eq!(scenario.owner.player.fishing_bobber.load(Relaxed), -1);
    assert_eq!(
        scenario.owner.player.inventory().held_item().get_damage(),
        2
    );
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 2);
    assert_eq!(sink.plugins[0].key, "item_interact_start");
    assert_eq!(sink.plugins[1].key, "item_interact_finish");
    assert_eq!(sink.plugins[1].damage, 2);
    assert_eq!(sink.order, ["plugin", "plugin", "dispatch", "dispatch"]);
    assert_eq!(sink.forwarded[0].event, GameEvent::ItemInteractFinish);
    assert_eq!(sink.forwarded[1].event, GameEvent::ItemInteractStart);
    for payload in &sink.forwarded {
        scenario.assert_source(payload.source.as_ref().unwrap(), DEPARTURE);
    }
}

struct Respawn {
    player: Arc<crate::entity::player::Player>,
    entered: AtomicBool,
}

impl EventHandler<GenericGameEvent> for Respawn {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !self.entered.swap(true, Relaxed) {
                self.player.respawn().await;
                use_item(&self.player, server);
            }
        })
    }
}

async fn respawn_reentry() {
    let mut scenario = Scenario::new(true);
    scenario.world.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.keep_inventory = true;
        info
    });
    scenario.rod(Some(true));
    scenario.capture(None);
    let player = scenario.owner.player.clone();
    let id = player.get_entity().entity_id;
    let uuid = player.get_entity().entity_uuid;
    let instance = Arc::as_ptr(&player);
    let life = player.living_entity.damage_lifecycle();
    player.set_respawn_point(
        scenario.world.dimension.clone(),
        BlockPos::new(8, 65, 8),
        0.0,
        0.0,
        true,
    );
    scenario
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(
            Arc::new(Respawn {
                player: player.clone(),
                entered: AtomicBool::new(false),
            }),
            EventPriority::High,
            true,
        );
    let server = scenario.server.clone();
    let actor = player.clone();
    let action = tokio::task::spawn_blocking(move || use_item(&actor, &server));
    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        scenario.owner.with_outgoing_writer(action),
    )
    .await;
    if completed.is_err() {
        player.client.java().unwrap().close();
    }
    completed
        .expect("callback respawn and nested action must complete")
        .unwrap();
    assert_eq!(Arc::as_ptr(&player), instance);
    assert_eq!(player.get_entity().entity_id, id);
    assert_eq!(player.get_entity().entity_uuid, uuid);
    assert_ne!(player.living_entity.damage_lifecycle(), life);
    assert!(scenario.world.contains_player_instance(&player));
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(
        sink.plugins.len(),
        2,
        "outer cast and new-life reel each notify once"
    );
    assert_eq!(
        sink.forwarded.len(),
        1,
        "stale outer event must not bind the new life"
    );
    assert_eq!(sink.forwarded[0].event, GameEvent::ItemInteractFinish);
    scenario.assert_source(
        sink.forwarded[0].source.as_ref().unwrap(),
        Vector3::new(8.5, 65.1, 8.5),
    );
    drop(sink);
    scenario.use_item(); // A later genuine cast in the same new life still forwards.
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 3);
    assert_eq!(sink.forwarded.len(), 2);
    scenario.assert_source(
        sink.forwarded[1].source.as_ref().unwrap(),
        Vector3::new(8.5, 65.1, 8.5),
    );
}

fn source_less_block() {
    let scenario = Scenario::new(true);
    scenario.capture(None);
    let position = BlockPos::new(8, 65, 8);
    let mut lever = LeverLikeProperties::default(&Block::LEVER);
    lever.face = AttachFace::Floor;
    scenario
        .world
        .level
        .loaded_chunks
        .get(&position.chunk_position())
        .unwrap()
        .set_block_absolute_y(8, 65, 8, lever.to_state_id(&Block::LEVER));
    scenario
        .owner
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 65.0, 10.5));
    scenario
        .owner
        .client()
        .handle_use_item_on(
            &scenario.owner.player,
            &SUseItemOn {
                hand: VarInt(0),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(3),
            },
            &scenario.server,
        )
        .unwrap();
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 1);
    assert_eq!(sink.plugins[0].key, "block_activate");
    assert_eq!(sink.forwarded.len(), 1);
    assert_eq!(sink.forwarded[0].event, GameEvent::BlockActivate);
    assert_eq!(sink.forwarded[0].position, Vector3::new(8.5, 65.5, 8.5));
    assert!(
        sink.forwarded[0].source.is_none(),
        "legacy block emission must not invent its interacting player"
    );
    drop(sink);
    scenario
        .world
        .emit_game_event("plugin:unknown_vibration", DEPARTURE);
    scenario.world.emit_game_event("minecraft:step", DEPARTURE);
    let sink = scenario.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 3);
    assert_eq!(sink.plugins[1].key, "plugin:unknown_vibration");
    assert_eq!(sink.plugins[2].key, "minecraft:step");
    assert_eq!(
        sink.forwarded.len(),
        2,
        "unknown raw key must not acquire a guessed builtin"
    );
    assert_eq!(sink.forwarded[1].event, GameEvent::Step);
    assert!(sink.forwarded[1].source.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s1_plugin_mutation_cancellation_and_reentry() {
    mutation("step", GameEvent::Step);
    mutation("minecraft:teleport", GameEvent::Teleport);
    for mode in 0..7 {
        suppressed(mode);
    }
    reentry();
    respawn_reentry().await;
    source_less_block();
}
