use super::*;
mod concurrency;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::player::player_change_world::PlayerChangeWorldEvent,
};
use pumpkin_data::packet::clientbound::play as outgoing;
use pumpkin_protocol::ser::{NetworkReadExt, NetworkReadSliceExt};
use pumpkin_util::text::TextComponent;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Default, PartialEq, Eq)]
enum DisplayName {
    #[default]
    Unchanged,
    Cleared,
    Named(String),
}

#[derive(Debug, Default)]
struct Info {
    uuid: Uuid,
    added: bool,
    mode: Option<i32>,
    listed: Option<bool>,
    latency: Option<i32>,
    display: DisplayName,
    order: Option<i32>,
}

// Decode the client-visible wire fields independently of the roster implementation.
fn infos(packet: &Bytes) -> Vec<Info> {
    let mut data = packet.as_ref();
    if data.get_var_int().unwrap().0 != outgoing::PLAYER_INFO_UPDATE.to_id(CURRENT_MC_VERSION) {
        return Vec::new();
    }
    let flags = data.get_u8().unwrap();
    let count = data.get_var_int().unwrap().0;
    let mut result = Vec::new();
    for _ in 0..count {
        let mut info = Info {
            uuid: data.get_uuid().unwrap(),
            ..Info::default()
        };
        if flags & 1 != 0 {
            info.added = true;
            let _ = data.get_str_borrowed().unwrap();
            let properties = data.get_var_int().unwrap().0;
            for _ in 0..properties {
                let _ = data.get_str_borrowed().unwrap();
                let _ = data.get_str_borrowed().unwrap();
                if data.get_bool().unwrap() {
                    let _ = data.get_str_borrowed().unwrap();
                }
            }
        }
        if flags & 2 != 0 && data.get_bool().unwrap() {
            let _ = data.get_uuid().unwrap();
            let _ = data.get_i64_be().unwrap();
            for _ in 0..2 {
                let length = data.get_var_int().unwrap().0 as usize;
                assert!(length <= data.len());
                data = &data[length..];
            }
        }
        if flags & 4 != 0 {
            info.mode = Some(data.get_var_int().unwrap().0);
        }
        if flags & 8 != 0 {
            info.listed = Some(data.get_bool().unwrap());
        }
        if flags & 16 != 0 {
            info.latency = Some(data.get_var_int().unwrap().0);
        }
        if flags & 32 != 0 {
            info.display = if data.get_bool().unwrap() {
                DisplayName::Named(data.get_component(&CURRENT_MC_VERSION).unwrap().get_text())
            } else {
                DisplayName::Cleared
            };
        }
        if flags & 64 != 0 {
            info.order = Some(data.get_var_int().unwrap().0);
        }
        if flags & 128 != 0 {
            assert!(data.get_bool().unwrap());
        }
        result.push(info);
    }
    assert!(data.is_empty(), "unparsed player-info bytes");
    result
}

fn removals(packets: &[Bytes], uuid: Uuid) -> usize {
    packets
        .iter()
        .map(|packet| {
            let mut data = packet.as_ref();
            if data.get_var_int().unwrap().0
                != outgoing::PLAYER_INFO_REMOVE.to_id(CURRENT_MC_VERSION)
            {
                return 0;
            }
            let count = data.get_var_int().unwrap().0;
            (0..count)
                .filter(|_| data.get_uuid().unwrap() == uuid)
                .count()
        })
        .sum()
}

fn added(packets: &[Bytes]) -> HashSet<Uuid> {
    packets
        .iter()
        .flat_map(infos)
        .filter(|p| p.added)
        .map(|p| p.uuid)
        .collect()
}

fn assert_initialized_before_spawns(packets: &[Bytes]) {
    let mut initialized = HashSet::new();
    for packet in packets {
        initialized.extend(
            infos(packet)
                .into_iter()
                .filter(|p| p.added)
                .map(|p| p.uuid),
        );
        for (_, uuid) in spawn_ids(std::slice::from_ref(packet)) {
            assert!(
                initialized.contains(&uuid),
                "Java player spawn precedes tab initialization"
            );
        }
    }
}

fn spawn_ids(packets: &[Bytes]) -> Vec<(i32, Uuid)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            (data.get_var_int().unwrap().0 == outgoing::ADD_ENTITY.to_id(CURRENT_MC_VERSION))
                .then(|| (data.get_var_int().unwrap().0, data.get_uuid().unwrap()))
        })
        .collect()
}

async fn transfer(scenario: &Scenario, session: &Session, dimension: &str) {
    let teleport = session.player.teleport_id_count.load(Ordering::Relaxed);
    scenario.command(&format!(
        "execute in minecraft:{dimension} run tp {} 3.5 64 3.5",
        session.player.gameprofile.name
    ));
    scenario
        .until("b1019_g17 transfer", || {
            session.player.world().dimension.minecraft_name == format!("minecraft:{dimension}")
                && session.player.teleport_id_count.load(Ordering::Relaxed) > teleport
                && session.is_idle()
        })
        .await;
    session.loaded(&scenario.server);
    scenario.ticks(2).await;
}

#[tokio::test]
async fn b1019_g17_login_sees_both_dimensions_and_announces_globally() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "ListA").await;
        let b = scenario.join(Uuid::new_v4(), "ListB").await;
        transfer(&scenario, &b, "the_nether").await;
        a.packets();
        b.packets();
        let c = scenario.join(Uuid::new_v4(), "ListC").await;
        let packets = c.packets();
        let expected = HashSet::from([
            a.player.gameprofile.id,
            b.player.gameprofile.id,
            c.player.gameprofile.id,
        ]);
        assert_eq!(
            added(&packets),
            expected,
            "issue #117: login omits another dimension"
        );
        assert_initialized_before_spawns(&packets);
        for observer in [&a, &b] {
            let packets = observer.packets();
            assert_initialized_before_spawns(&packets);
            assert!(
                added(&packets).contains(&c.player.gameprofile.id),
                "issue #117: remote observer missed new login"
            );
        }
        let spawns = spawn_ids(&packets);
        assert!(
            spawns.iter().any(|(_, id)| *id == a.player.gameprofile.id),
            "same-world entity control"
        );
        assert!(
            spawns.iter().all(|(_, id)| *id != b.player.gameprofile.id),
            "foreign entity leaked into world"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1019_g17_transfer_keeps_entries_and_initializes_before_spawn() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "MoveA").await;
        let b = scenario.join(Uuid::new_v4(), "MoveB").await;
        transfer(&scenario, &b, "the_nether").await;
        let c = scenario.join(Uuid::new_v4(), "MoveC").await;
        let initial = added(&b.packets());
        a.packets();
        c.packets();
        transfer(&scenario, &a, "the_nether").await;
        let destination = b.packets();
        assert_eq!(
            removals(&c.packets(), a.player.gameprofile.id),
            0,
            "issue #117: transfer removes online tab entry"
        );
        assert_eq!(removals(&destination, a.player.gameprofile.id), 0);
        let spawns = spawn_ids(&destination);
        assert!(
            spawns.iter().any(|(_, id)| *id == a.player.gameprofile.id),
            "destination entity not spawned"
        );
        for (_, id) in spawns {
            assert!(
                initial.contains(&id),
                "Java spawn preceded player initialization"
            );
        }
        assert!(
            scenario
                .world(&Dimension::OVERWORLD)
                .entity_tracker
                .get_tracked_entity(a.player.entity_id())
                .is_none()
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1019_g17_logout_reconnect_removes_once_without_retiring_new_session() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "ExitA").await;
        let id = Uuid::new_v4();
        let old = scenario.join(id, "ExitB").await;
        transfer(&scenario, &old, "the_nether").await;
        let c = scenario.join(Uuid::new_v4(), "ExitC").await;
        a.packets();
        c.packets();
        scenario.leave(&old).await;
        for observer in [&a, &c] {
            assert_eq!(
                removals(&observer.packets(), id),
                1,
                "issue #117: real logout must remove globally once"
            );
        }
        let new = scenario.join(id, "ExitB").await;
        for observer in [&a, &c] {
            assert!(added(&observer.packets()).contains(&id));
        }
        // A delayed old-instance removal must not withdraw the replacement's entry.
        scenario
            .drive(
                "stale detach",
                new.player.world().remove_player(&old.player, true),
            )
            .await;
        assert!(
            scenario
                .server
                .get_all_players()
                .iter()
                .any(|p| Arc::ptr_eq(p, &new.player)),
            "stale logout removed rejoined session"
        );
        for observer in [&a, &c] {
            assert_eq!(removals(&observer.packets(), id), 0);
        }
        scenario.leave(&new).await;
        for observer in [&a, &c] {
            assert_eq!(removals(&observer.packets(), id), 1);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1019_g17_metadata_updates_same_and_other_dimensions() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "MetaA").await;
        let b = scenario.join(Uuid::new_v4(), "MetaB").await;
        let c = scenario.join(Uuid::new_v4(), "MetaC").await;
        transfer(&scenario, &c, "the_nether").await;
        for observer in [&a, &b, &c] {
            observer.packets();
        }
        scenario.command("gamemode creative MetaB");
        b.player.set_tab_list_listed(false);
        b.player.set_tab_list_latency(123);
        b.player.set_tab_list_order(7);
        b.player
            .set_display_name(Some(TextComponent::text("Display")));
        b.player
            .set_tab_list_name(Some(TextComponent::text("Custom")));
        for observer in [&a, &b, &c] {
            let rows: Vec<_> = observer
                .packets()
                .iter()
                .flat_map(infos)
                .filter(|p| p.uuid == b.player.gameprofile.id)
                .collect();
            assert!(
                rows.iter().any(|p| p.mode == Some(1)),
                "issue #117: gamemode update missed observer"
            );
            assert!(
                rows.iter().any(|p| p.listed == Some(false)),
                "issue #117: listed update missed observer"
            );
            assert!(rows.iter().any(|p| p.latency == Some(123)));
            assert!(rows.iter().any(|p| p.order == Some(7)));
            for name in ["Display", "Custom"] {
                assert!(
                    rows.iter()
                        .any(|p| matches!(&p.display, DisplayName::Named(value) if value == name)),
                    "display update missed observer"
                );
            }
        }
        let d = scenario.join(Uuid::new_v4(), "MetaD").await;
        let row = d
            .packets()
            .iter()
            .flat_map(infos)
            .find(|p| p.uuid == b.player.gameprofile.id && p.added)
            .unwrap();
        assert_eq!(
            (row.mode, row.listed, row.latency, row.order),
            (Some(1), Some(false), Some(123), Some(7))
        );
        assert_eq!(row.display, DisplayName::Named("Custom".into()));
        scenario.stop().await;
    })
    .await;
}

struct TransferGate {
    uuid: Uuid,
    cancel: bool,
    entered: AtomicBool,
    release: tokio::sync::Notify,
}

#[tokio::test]
async fn b1019_g17_display_only_name_survives_later_login() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "NameA").await;
        let b = scenario.join(Uuid::new_v4(), "NameB").await;
        transfer(&scenario, &a, "the_nether").await;
        b.player
            .set_display_name(Some(TextComponent::text("Display only")));
        let c = scenario.join(Uuid::new_v4(), "NameC").await;
        let rows = c.packets();
        let row = rows
            .iter()
            .flat_map(infos)
            .find(|p| p.uuid == b.player.gameprofile.id && p.added)
            .unwrap();
        assert_eq!(row.display, DisplayName::Named("Display only".into()));
        a.packets();
        b.player.set_tab_list_name(None);
        assert!(
            a.packets()
                .iter()
                .flat_map(infos)
                .any(|p| p.display == DisplayName::Named("Display only".into()))
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn b1019_g17_same_world_control() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "ControlA").await;
        let b = scenario.join(Uuid::new_v4(), "ControlB").await;
        let c = scenario.join(Uuid::new_v4(), "ControlC").await;
        assert_eq!(added(&c.packets()).len(), 3);
        a.packets();
        b.player.set_tab_list_listed(false);
        assert!(
            a.packets()
                .iter()
                .flat_map(infos)
                .any(|p| p.uuid == b.player.gameprofile.id && p.listed == Some(false))
        );
        scenario.leave(&b).await;
        assert_eq!(removals(&a.packets(), b.player.gameprofile.id), 1);
        scenario.stop().await;
    })
    .await;
}

struct ArrivalGate {
    entered: AtomicBool,
    release: tokio::sync::Notify,
}
impl EventHandler<crate::plugin::world::chunk_send::ChunkSend> for ArrivalGate {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::world::chunk_send::ChunkSend,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.world.dimension == Dimension::THE_NETHER
                && !self.entered.swap(true, Ordering::AcqRel)
            {
                tokio::time::timeout(Duration::from_secs(30), self.release.notified())
                    .await
                    .unwrap();
            }
        })
    }
}

#[tokio::test]
async fn b1019_g17_disconnect_during_accepted_transfer_removes_once() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "ArrivalA").await;
        let b = scenario.join(Uuid::new_v4(), "ArrivalB").await;
        transfer(&scenario, &b, "the_nether").await;
        let c = scenario.join(Uuid::new_v4(), "ArrivalC").await;
        let gate = Arc::new(ArrivalGate {
            entered: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        scenario
            .server
            .plugin_manager
            .register(gate.clone(), EventPriority::Normal, true);
        for observer in [&a, &b, &c] {
            observer.packets();
        }
        scenario.command("execute in minecraft:the_nether run tp ArrivalA 3.5 64 3.5");
        scenario
            .until("accepted transfer chunk gate", || {
                gate.entered.load(Ordering::Acquire)
            })
            .await;
        assert_eq!(a.player.world().dimension, Dimension::THE_NETHER);
        a.client().close();
        gate.release.notify_one();
        scenario.leave(&a).await;
        for observer in [&b, &c] {
            assert_eq!(
                removals(&observer.packets(), a.player.gameprofile.id),
                1,
                "disconnect during transfer must remove once everywhere"
            );
        }
        assert_eq!(scenario.server.get_all_players().len(), 2);
        for world in scenario.server.worlds.load_full().iter() {
            assert!(
                world
                    .entity_tracker
                    .get_tracked_entity(a.player.entity_id())
                    .is_none()
            );
        }
        scenario.stop().await;
    })
    .await;
}
impl EventHandler<PlayerChangeWorldEvent> for TransferGate {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.player.gameprofile.id != self.uuid {
                return;
            }
            self.entered.store(true, Ordering::Release);
            if self.cancel {
                event.cancelled = true;
                return;
            }
            tokio::time::timeout(Duration::from_secs(30), self.release.notified())
                .await
                .unwrap();
        })
    }
}

#[tokio::test]
async fn b1019_g17_cancelled_and_disconnected_transfer_do_not_republish() {
    run(async {
        let scenario = Scenario::new().await;
        let a = scenario.join(Uuid::new_v4(), "GateA").await;
        let b = scenario.join(Uuid::new_v4(), "GateB").await;
        transfer(&scenario, &b, "the_nether").await;
        let c = scenario.join(Uuid::new_v4(), "GateC").await;
        let cancel = Arc::new(TransferGate {
            uuid: c.player.gameprofile.id,
            cancel: true,
            entered: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        scenario
            .server
            .plugin_manager
            .register(cancel.clone(), EventPriority::Normal, true);
        a.packets();
        b.packets();
        c.packets();
        scenario.command("execute in minecraft:the_nether run tp GateC 3.5 64 3.5");
        scenario
            .until("cancelled transfer", || {
                cancel.entered.load(Ordering::Acquire) && c.is_idle()
            })
            .await;
        assert_eq!(c.player.world().dimension, Dimension::OVERWORLD);
        for observer in [&a, &b] {
            assert_eq!(removals(&observer.packets(), c.player.gameprofile.id), 0);
        }
        let gate = Arc::new(TransferGate {
            uuid: a.player.gameprofile.id,
            cancel: false,
            entered: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        scenario
            .server
            .plugin_manager
            .register(gate.clone(), EventPriority::Normal, true);
        scenario.command("execute in minecraft:the_nether run tp GateA 3.5 64 3.5");
        scenario
            .until("transfer event gate", || {
                gate.entered.load(Ordering::Acquire)
            })
            .await;
        a.client().close();
        gate.release.notify_one();
        scenario.leave(&a).await;
        for observer in [&b, &c] {
            let packets = observer.packets();
            assert_eq!(removals(&packets, a.player.gameprofile.id), 1);
            assert!(
                !added(&packets).contains(&a.player.gameprofile.id),
                "failed transfer republishes disconnected session"
            );
        }
        assert_eq!(scenario.server.get_all_players().len(), 2);
        scenario.stop().await;
    })
    .await;
}
