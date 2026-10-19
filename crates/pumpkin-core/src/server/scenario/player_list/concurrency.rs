use super::*;
use crate::plugin::player::player_login::PlayerLoginEvent;
use crate::plugin::server::packet::PacketSentEvent;
use pumpkin_protocol::ser::NetworkWriteExt;
use pumpkin_protocol::{VarInt, java::server::play::SClientCommand};
use pumpkin_util::version::JavaMinecraftVersion;

struct DetachGate {
    entity_id: i32,
    observer: Uuid,
    entered: AtomicBool,
    release: tokio::sync::Notify,
}

struct LogoutOnInitialization {
    leaving: Arc<Player>,
    fired: AtomicBool,
}

impl EventHandler<PlayerLoginEvent> for LogoutOnInitialization {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerLoginEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.player.gameprofile.name == "RaceD" {
                event
                    .player
                    .client
                    .java()
                    .unwrap()
                    .version
                    .store(JavaMinecraftVersion::V_1_21_4);
            }
        })
    }
}

impl EventHandler<PacketSentEvent> for LogoutOnInitialization {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.player.gameprofile.name != "RaceD"
                || event.packet_id != outgoing::PLAYER_INFO_UPDATE.to_id(CURRENT_MC_VERSION)
            {
                return;
            }
            let mut packet = Vec::new();
            packet.write_var_int(&VarInt(event.packet_id)).unwrap();
            packet.extend_from_slice(&event.payload);
            if infos(&packet.into())
                .iter()
                .any(|row| row.added && row.uuid == self.leaving.gameprofile.id)
                && !self.fired.swap(true, Ordering::AcqRel)
            {
                tokio::time::timeout(
                    Duration::from_secs(30),
                    crate::net::java::connection_owner::finish_disconnect(&self.leaving, server),
                )
                .await
                .unwrap();
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g17_logout_during_initialization_cannot_leave_ghost_entry() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "RaceA").await;
        let b = scenario.join(Uuid::new_v4(), "RaceB").await;
        transfer(&scenario, &a, "the_nether").await;
        let hook = Arc::new(LogoutOnInitialization {
            leaving: b.player.clone(),
            fired: AtomicBool::new(false),
        });
        scenario
            .server
            .plugin_manager
            .register::<PlayerLoginEvent, _>(hook.clone(), EventPriority::Normal, true);
        scenario
            .server
            .plugin_manager
            .register::<PacketSentEvent, _>(hook.clone(), EventPriority::Normal, true);
        a.packets();
        let d = scenario.join(Uuid::new_v4(), "RaceD").await;
        assert!(
            hook.fired.load(Ordering::Acquire),
            "logout hook did not run during initializing ADD"
        );
        let packets = d.packets();
        let mut present = false;
        for packet in &packets {
            if infos(packet)
                .iter()
                .any(|row| row.added && row.uuid == b.player.gameprofile.id)
            {
                present = true;
            }
            if removals(std::slice::from_ref(packet), b.player.gameprofile.id) != 0 {
                present = false;
            }
        }
        assert!(
            !present,
            "issue #117: stale initialization resurrected logged-out tab entry"
        );
        assert_eq!(removals(&packets, b.player.gameprofile.id), 1);
        assert_eq!(removals(&a.packets(), b.player.gameprofile.id), 1);
        scenario.stop().await;
    })
    .await;
}

impl EventHandler<PacketSentEvent> for DetachGate {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.player.gameprofile.id != self.observer
                || event.packet_id != outgoing::REMOVE_ENTITIES.to_id(CURRENT_MC_VERSION)
            {
                return;
            }
            let mut data = event.payload.as_ref();
            let count = data.get_var_int().unwrap().0;
            let removes_player =
                (0..count).any(|_| data.get_var_int().unwrap().0 == self.entity_id);
            if removes_player && !self.entered.swap(true, Ordering::AcqRel) {
                tokio::time::timeout(Duration::from_secs(30), self.release.notified())
                    .await
                    .unwrap();
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g17_login_during_respawn_detach_sees_online_session() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "GapA").await;
        let b = scenario.join(Uuid::new_v4(), "GapB").await;
        transfer(&scenario, &a, "the_nether").await;
        transfer(&scenario, &b, "the_nether").await;
        let c = scenario.join(Uuid::new_v4(), "GapC").await;
        let gate = Arc::new(DetachGate {
            entity_id: a.player.entity_id(),
            observer: b.player.gameprofile.id,
            entered: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        // Exercise the real multiversion packet hook without rewriting its 26.3 payload.
        b.client().version.store(JavaMinecraftVersion::V_1_21_4);
        scenario
            .server
            .plugin_manager
            .register(gate.clone(), EventPriority::Normal, true);
        scenario.command("kill GapA");
        a.send(
            &scenario.server,
            &SClientCommand::new(VarInt(SClientCommand::PERFORM_RESPAWN)),
        );
        scenario
            .until("respawn world detach", || {
                gate.entered.load(Ordering::Acquire)
            })
            .await;
        assert!(
            !scenario
                .server
                .get_all_players()
                .iter()
                .any(|p| Arc::ptr_eq(p, &a.player)),
            "gate must cover actual world-membership gap"
        );
        let d = scenario.join(Uuid::new_v4(), "GapD").await;
        let present_at_login = added(&d.packets()).contains(&a.player.gameprofile.id);
        gate.release.notify_one();
        scenario
            .until("respawn completion", || {
                a.is_idle() && scenario.server.tasks.is_empty()
            })
            .await;
        a.loaded(&scenario.server);
        assert_eq!(a.player.world().dimension, Dimension::OVERWORLD);
        assert!(
            present_at_login,
            "issue #117: detached online session missing at login"
        );
        c.packets();
        d.packets();
        b.packets();
        scenario.leave(&a).await;
        for observer in [&b, &c, &d] {
            assert_eq!(removals(&observer.packets(), a.player.gameprofile.id), 1);
        }
        scenario.stop().await;
    })
    .await;
}
