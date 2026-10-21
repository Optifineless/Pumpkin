use super::{items::ground, support::*};
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::{ClientPlatform, PlayerConfig, java::combat_test_support::captured_client},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::{entity_spawn::EntitySpawnEvent, projectile_launch::ProjectileLaunchEvent},
    },
    server::Server,
};
use pumpkin_data::{Block, biome::Biome, damage::DamageType, dimension::Dimension, item::Item};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::math::vector3::Vector3;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug)]
enum Transition {
    DeathRespawn(bool),
    DisconnectRejoin,
    WorldReplacement,
    LifeReplacement,
}

struct LifecycleHook {
    player: Arc<Player>,
    transition: Transition,
    stage: Veto,
    fired: AtomicBool,
    candidate: AtomicI32,
    replacement: Mutex<Option<Arc<Player>>>,
}

impl LifecycleHook {
    async fn transition(&self, server: &Arc<Server>) {
        if self.fired.swap(true, SeqCst) {
            return;
        }
        let p = &self.player;
        assert_owner_released(p.inventory().slot_transaction(0));
        assert_reservations_released(p.world());
        match self.transition {
            Transition::DeathRespawn(_) => {
                assert!(p.damage_with_context(
                    &**p,
                    f32::MAX,
                    DamageType::GENERIC_KILL,
                    None,
                    None,
                    None
                ));
                assert!(!p.living_entity.is_alive());
                assert!(p.world().respawn_player_with_outcome(p, false, false).await);
                assert!(p.living_entity.is_alive());
                p.set_client_loaded(true);
            }
            Transition::DisconnectRejoin => {
                crate::net::java::connection_owner::finish_disconnect(p, server).await;
                let profile = p.gameprofile.clone();
                let (client, _packets) = captured_client(profile.clone());
                let (replacement, _) = server
                    .add_player(
                        Arc::new(ClientPlatform::Java(client)),
                        profile,
                        Some(PlayerConfig::default()),
                    )
                    .await
                    .unwrap();
                replacement.set_client_loaded(true);
                *self.replacement.lock().unwrap() = Some(replacement);
            }
            Transition::WorldReplacement => {
                p.get_entity()
                    .set_world(server.get_world_from_dimension(&Dimension::THE_NETHER));
            }
            Transition::LifeReplacement => p.living_entity.reset_state(),
        }
    }
}

impl EventHandler<ProjectileLaunchEvent> for LifecycleHook {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.shooter_id == Some(self.player.entity_id()) {
                self.candidate.store(event.entity_id, SeqCst);
                if self.stage == Veto::Launch {
                    self.transition(server).await;
                }
            }
        })
    }
}

impl EventHandler<EntitySpawnEvent> for LifecycleHook {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if self.stage == Veto::Spawn && event.entity_id == self.candidate.load(SeqCst) {
                self.transition(server).await;
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_callback_life_and_session_replacement_abort_payment() {
    for transition in [
        Transition::DeathRespawn(false),
        Transition::DeathRespawn(true),
        Transition::DisconnectRejoin,
        Transition::WorldReplacement,
        Transition::LifeReplacement,
    ] {
        for stage in [Veto::Launch, Veto::Spawn] {
            tokio::time::timeout(Duration::from_secs(90), transition_case(transition, stage))
                .await
                .unwrap();
        }
    }
}

async fn transition_case(transition: Transition, stage: Veto) {
    let f = DeathTestWorld::new().await;
    let world = f.world();
    crate::world::spawn_test_support::publish(
        &world,
        crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    f.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = 8;
        info.spawn_y = 64;
        info.spawn_z = 8;
        info
    });
    if let Transition::DeathRespawn(keep) = transition {
        f.keep_inventory(keep);
    }
    let mut p = player(&f);
    p.player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    p.player.inventory().set_stack(0, stack(2, &Item::SNOWBALL));
    let (session, _) = f
        .server
        .player_data_storage
        .load_data(&p.player.gameprofile.id)
        .await
        .unwrap();
    *p.player.storage_session.lock().unwrap() = Some(session);
    let life = p.player.living_entity.damage_lifecycle();
    let hook = Arc::new(LifecycleHook {
        player: p.player.clone(),
        transition,
        stage,
        fired: AtomicBool::new(false),
        candidate: AtomicI32::new(-1),
        replacement: Mutex::new(None),
    });
    f.server
        .plugin_manager
        .register::<ProjectileLaunchEvent, _>(hook.clone(), EventPriority::Normal, true);
    f.server.plugin_manager.register::<EntitySpawnEvent, _>(
        hook.clone(),
        EventPriority::Normal,
        true,
    );
    let recv = dispatch(p.player.clone(), f.server.clone());
    let result = p.with_outgoing_writer(recv).await.unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    assert!(
        hook.fired.load(SeqCst),
        "callback transition was not reached"
    );
    assert!(
        world
            .get_entity_by_id(hook.candidate.load(SeqCst))
            .is_none(),
        "interrupted life published a projectile: {transition:?}, {stage:?}"
    );
    if matches!(transition, Transition::DisconnectRejoin) {
        let replacement = hook.replacement.lock().unwrap().clone().unwrap();
        assert_eq!(loose(&replacement, &Item::SNOWBALL), 2);
        assert!(world.contains_player_instance(&replacement));
        assert!(!world.contains_player_instance(&p.player));
    } else {
        assert_eq!(
            loose(&p.player, &Item::SNOWBALL) + ground(&p.player, &Item::SNOWBALL),
            2,
            "interrupted source lost or duplicated stock: {transition:?}, {stage:?}"
        );
    }
    if matches!(
        transition,
        Transition::DeathRespawn(_) | Transition::LifeReplacement
    ) {
        assert_ne!(p.player.living_entity.damage_lifecycle(), life);
    }
    f.server.shutdown().await;
}

fn dispatch(
    source: Arc<Player>,
    server: Arc<Server>,
) -> tokio::sync::oneshot::Receiver<std::thread::Result<()>> {
    let runtime = tokio::runtime::Handle::current();
    let (send, recv) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let ClientPlatform::Java(client) = source.client.as_ref() {
                client.handle_use_item(
                    &source,
                    &SUseItem {
                        hand: VarInt(0),
                        sequence: VarInt(1),
                        yaw: 0.0,
                        pitch: 0.0,
                    },
                    &server,
                );
            }
        }));
        let _ = send.send(result);
    });
    recv
}
