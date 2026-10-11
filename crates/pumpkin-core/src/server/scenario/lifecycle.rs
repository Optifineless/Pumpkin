use super::*;
use crate::{
    entity::ai::goal::{active_target::ActiveTargetGoal, goal_selector::GoalSelector},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_spawn_location::PlayerSpawnLocationEvent,
    },
};
use pumpkin_data::{damage::DamageType, item::Item, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SClientCommand};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Use the real target selector with its random search interval disabled for repeatable ticks.
pub(super) fn targeting_mob(world: &Arc<World>, session: &Session) -> Arc<dyn EntityBase> {
    let entity = Scenario::mob(world, &EntityType::ZOMBIE, Vector3::new(11.5, 64.0, 3.5));
    let mob = entity.get_mob().unwrap().get_mob_entity();
    *mob.target_selector.lock().unwrap() = GoalSelector::default();
    mob.add_target_goal(
        0,
        ActiveTargetGoal::new(
            mob,
            &EntityType::PLAYER,
            0,
            true,
            false,
            None::<fn(&crate::entity::living::LivingEntity, &World) -> bool>,
        ),
    );
    // Acquisition happens in the real mob tick, never by assigning the target field.
    assert!(mob.get_target().is_none());
    assert!(session.player.has_client_loaded());
    entity
}

#[tokio::test]
async fn s1_rejoin_replaces_instance_and_retires_old_publications() {
    run(async {
        // PlayerList.placeNewPlayer/remove; old sessions disappear from ChunkMap and player ticks.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let old = scenario.join(id, "Rejoin").await;
        scenario.ticks(2).await;
        scenario.leave(&old).await;
        assert!(old.client().is_closed());
        assert!(
            crate::net::java::combat_test_support::published_player(old.client()).is_none(),
            "old player remains published by the connection"
        );
        old.packets();
        let rejoined = scenario.join(id, "Rejoin").await;
        assert!(!Arc::ptr_eq(&old.player, &rejoined.player));
        let timeout = old.player.client_loaded_timeout.load(Ordering::Relaxed);
        let tick_counter = old.player.tick_counter.load(Ordering::Relaxed);
        scenario.ticks(3).await;
        for world in scenario.server.worlds.load().iter() {
            assert!(
                world
                    .players
                    .load()
                    .iter()
                    .all(|player| !Arc::ptr_eq(player, &old.player)),
                "old player remains published"
            );
            assert!(
                world
                    .entity_tracker
                    .get_tracked_entity(old.player.entity_id())
                    .is_none(),
                "old player remains tracked"
            );
        }
        assert_eq!(scenario.server.get_all_players().len(), 1);
        assert_eq!(
            old.player.client_loaded_timeout.load(Ordering::Relaxed),
            timeout,
            "old player still ticks"
        );
        assert_eq!(
            old.player.tick_counter.load(Ordering::Relaxed),
            tick_counter,
            "old instance still ticks"
        );
        assert!(old.packets().is_empty(), "packets sent to retired client");
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "known bug: audit row 7 / tester R31"]
async fn s1_logout_drops_hostile_target() {
    run(async {
        // PlayerList.remove -> Entity.setRemoved(UNLOADED_WITH_PLAYER), TargetGoal.canContinueToUse.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "Target").await;
        let observer = scenario.join(Uuid::new_v4(), "Observer").await;
        scenario.command("gamemode spectator Observer");
        let zombie = targeting_mob(&player.player.world(), &player);
        scenario.ticks(4).await;
        assert_eq!(
            zombie
                .get_mob()
                .unwrap()
                .get_mob_entity()
                .get_target()
                .unwrap()
                .get_entity()
                .entity_id,
            player.player.entity_id(),
            "hostile must acquire target before logout"
        );
        scenario.leave(&player).await;
        let reason = player.player.get_entity().removal_reason.load();
        scenario.ticks(8).await;
        let dropped = zombie
            .get_mob()
            .unwrap()
            .get_mob_entity()
            .get_target()
            .is_none();
        scenario.leave(&observer).await;
        scenario.stop().await;
        assert!(
            dropped,
            "audit row 7: hostile retains the logged-out player as a live target"
        );
        assert!(
            matches!(
                reason,
                Some(crate::entity::RemovalReason::UnloadedWithPlayer)
            ),
            "audit row 7: logout did not retire the old entity"
        );
    })
    .await;
}

fn die(scenario: &Scenario, session: &Session) -> Vec<Arc<dyn EntityBase>> {
    session
        .player
        .get_entity()
        .set_pos(Vector3::new(11.5, 64.0, 11.5));
    session
        .player
        .inventory
        .set_slot(0, ItemStack::new(3, &Item::DIAMOND));
    scenario.command(&format!("kill {}", session.player.gameprofile.name));
    assert_eq!(session.player.living_entity.health.load(), 0.0);
    session
        .player
        .world()
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_item_entity().is_some())
        .cloned()
        .collect()
}

async fn respawn(scenario: &Scenario, session: &Session) {
    session.send(
        &scenario.server,
        &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
    );
    scenario
        .until("respawn packet task", || scenario.server.tasks.is_empty())
        .await;
    assert!(
        session.player.living_entity.health.load() > 0.0,
        "respawn task did not restore health"
    );
    session.confirm_teleport(&scenario.server);
}

#[tokio::test]
async fn s2_death_drops_stay_at_death_position_after_respawn() {
    run(async {
        // Player.dropEquipment/Inventory.dropAll and PlayerList.respawn/ServerPlayer.restoreFrom.
        let scenario = Scenario::new().await;
        let session = scenario.join(Uuid::new_v4(), "Drops").await;
        let drops = die(&scenario, &session);
        assert_eq!(drops.len(), 1);
        let death_position = drops[0].get_entity().pos.load();
        assert!((death_position.x - 11.5).abs() < 2.0);
        assert!((death_position.z - 11.5).abs() < 2.0);
        // Freeze random throw motion so the scenario isolates respawn/pickup, not item scattering.
        drops[0]
            .get_entity()
            .velocity
            .store(Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(
            drops[0]
                .get_item_entity()
                .unwrap()
                .get_item_stack()
                .lock()
                .unwrap()
                .item_count,
            3
        );
        respawn(&scenario, &session).await;
        session.loaded(&scenario.server);
        scenario.ticks(30).await;
        assert_eq!(session.player.position(), SPAWN);
        assert!(
            session.player.inventory.get_slot(0).is_empty(),
            "respawn player picked up remote death drops"
        );
        assert!(!drops[0].get_entity().is_removed());
        let position = drops[0].get_entity().pos.load();
        assert_eq!(
            (position.x, position.z),
            (death_position.x, death_position.z),
            "death drops moved to respawn"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "known bug: audit row 9"]
async fn s2_loading_protection_restarts_after_time_on_death_screen() {
    run(async {
        // ServerGamePacketListenerImpl.handleClientCommand -> restartClientLoadTimerAfterRespawn (60).
        let scenario = Scenario::new().await;
        let session = scenario.join(Uuid::new_v4(), "Protection").await;
        die(&scenario, &session);
        scenario.ticks(70).await;
        respawn(&scenario, &session).await;
        let timer = session.player.client_loaded_timeout.load(Ordering::Relaxed);
        let protected = !session.player.has_client_loaded();
        let health = session.player.living_entity.health.load();
        session
            .player
            .living_entity
            .damage(session.player.as_ref(), 2.0, DamageType::GENERIC);
        let after = session.player.living_entity.health.load();
        scenario.stop().await;
        assert_eq!(
            timer, 60,
            "audit row 9: respawn loading timer was spent on the death screen"
        );
        assert!(protected);
        assert_eq!(
            after, health,
            "damage admitted before client finished loading"
        );
    })
    .await;
}

#[tokio::test]
async fn s2_disconnect_on_death_screen_then_rejoin_has_one_instance() {
    run(async {
        // PlayerList.remove/placeNewPlayer remove the old session even when health is zero.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let old = scenario.join(id, "DeathScreen").await;
        die(&scenario, &old);
        scenario.leave(&old).await;
        let new = scenario.join(id, "DeathScreen").await;
        scenario.ticks(2).await;
        assert!(!Arc::ptr_eq(&old.player, &new.player));
        assert_eq!(scenario.server.get_all_players().len(), 1);
        assert!(
            scenario
                .world(&Dimension::OVERWORLD)
                .entity_tracker
                .get_tracked_entity(old.player.entity_id())
                .is_none()
        );
        scenario.stop().await;
    })
    .await;
}

struct PauseRespawn {
    reached: AtomicBool,
    release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}
impl EventHandler<PlayerSpawnLocationEvent> for PauseRespawn {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerSpawnLocationEvent,
    ) -> BoxFuture<'a, ()> {
        let release = self.release.lock().unwrap().take();
        self.reached.store(true, Ordering::Release);
        Box::pin(async move {
            if let Some(release) = release {
                release.await.unwrap();
            }
        })
    }
}

#[tokio::test]
#[ignore = "known bug: audit row 15"]
async fn s2_disconnect_during_respawn_does_not_republish_closed_instance() {
    run(async {
        // PlayerList.respawn/remove are serialized in vanilla; explicitly hold Pumpkin's task
        // before transfer, finish disconnect/rejoin, then release it. No scheduling race needed.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let old = scenario.join(id, "PendingRespawn").await;
        die(&scenario, &old);
        old.player.set_respawn_point(
            Dimension::THE_NETHER,
            BlockPos::new(3, 64, 3),
            0.0,
            0.0,
            true,
        );
        let (release, receiver) = tokio::sync::oneshot::channel();
        let hook = Arc::new(PauseRespawn {
            reached: AtomicBool::new(false),
            release: Mutex::new(Some(receiver)),
        });
        scenario
            .server
            .plugin_manager
            .register(hook.clone(), EventPriority::Normal, true);
        old.send(
            &scenario.server,
            &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
        );
        scenario
            .until("respawn reaches transfer barrier", || {
                hook.reached.load(Ordering::Acquire)
            })
            .await;
        scenario.leave(&old).await;
        let new = scenario.join(id, "PendingRespawn").await;
        let _ = release.send(()); // A fixed disconnect may already have cancelled the respawn.
        scenario
            .until("released respawn task", || scenario.server.tasks.is_empty())
            .await;
        scenario.ticks(2).await;
        let players = scenario.server.get_all_players();
        let valid = players.len() == 1 && Arc::ptr_eq(&players[0], &new.player);
        // An obsolete task may have replaced the current session. Clean both storage gates.
        scenario.leave(&new).await;
        scenario.stop().await;
        assert!(
            valid,
            "audit row 15: stale respawn republished the closed player or removed the rejoined session"
        );
    })
    .await;
}
