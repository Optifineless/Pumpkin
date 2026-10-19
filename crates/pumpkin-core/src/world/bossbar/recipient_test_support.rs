#![expect(
    clippy::unwrap_used,
    reason = "Production lifecycle fixture assertions"
)]

use crate::{
    plugin::{EventHandler, EventPriority, server::packet::PacketSentEvent},
    server::{
        Server, combat_test_support,
        scenario::{Scenario, Session},
    },
    world::World,
};
use futures::future::BoxFuture;
use pumpkin_data::{
    dimension::Dimension,
    packet::{CURRENT_MC_VERSION, clientbound::play::BOSS_EVENT},
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::{
    generation::generator::{WorldGenerator, flat::FlatGenerator},
    level::Level,
};
use std::{rc::Rc, sync::Arc};
use uuid::Uuid;

#[derive(Default)]
pub struct CancelTransfer(pub std::sync::atomic::AtomicBool);

impl EventHandler<crate::plugin::api::events::player::player_change_world::PlayerChangeWorldEvent>
    for CancelTransfer
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::player_change_world::PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            event.cancelled = self.0.load(std::sync::atomic::Ordering::Relaxed);
        })
    }
}

pub struct Fixture {
    pub scenario: Scenario,
    pub end: Arc<World>,
}

impl Fixture {
    pub async fn new() -> Self {
        let scenario = Scenario::new().await;
        let end = Arc::new(World::load(
            Level::from_root_folder(
                &pumpkin_config::world::LevelConfig::default(),
                scenario.directory.path().to_owned(),
                0,
                Dimension::THE_END,
            ),
            scenario.server.level_info.clone(),
            Dimension::THE_END,
            scenario.server.block_registry.clone(),
            Arc::downgrade(&scenario.server),
        ));
        let portal: Arc<dyn pumpkin_world::world::WorldPortalExt> =
            Arc::new(crate::world::WorldPortal(end.clone()));
        end.level.world_portal.store(Arc::new(Some(portal)));
        end.level
            .set_world_gen(Arc::new(WorldGenerator::Flat(Box::new(
                FlatGenerator::new(
                    end.level.seed,
                    Dimension::THE_END,
                    Vec::new(),
                    pumpkin_data::biome::Biome::PLAINS.registry_id.to_owned(),
                ),
            ))));
        scenario.server.worlds.rcu(|worlds| {
            let mut worlds = (**worlds).clone();
            worlds.push(end.clone());
            worlds
        });
        scenario.prepare_area(&end, Vector2::new(0, 0)).await;
        Self { scenario, end }
    }

    pub fn arena(&self) {
        for x in -3..=3 {
            for z in -3..=3 {
                let pos = Vector2::new(x, z);
                if !self.end.level.is_chunk_loaded(&pos) {
                    combat_test_support::publish_empty_chunk(&self.end, pos);
                }
            }
        }
    }

    pub async fn viewer(&self, id: Uuid, name: &str, world: &Arc<World>) -> Rc<Session> {
        let viewer = self.scenario.join(id, name).await;
        if !Arc::ptr_eq(&viewer.player.world(), world) {
            transfer(
                &self.scenario,
                &viewer,
                world,
                Vector3::new(3.5, 128.0, 3.5),
            )
            .await;
        }
        viewer.packets();
        viewer
    }
}

pub async fn transfer(
    scenario: &Scenario,
    viewer: &Session,
    destination: &Arc<World>,
    position: Vector3<f64>,
) {
    let source = viewer.player.world();
    let teleport = viewer
        .player
        .teleport_id_count
        .load(std::sync::atomic::Ordering::Relaxed);
    scenario
        .drive(
            "bossbar committed world teleport",
            viewer
                .player
                .teleport_world(destination.clone(), position, None, None),
        )
        .await;
    assert!(
        Arc::ptr_eq(&viewer.player.world(), destination),
        "teleport was cancelled before committing"
    );
    assert!(
        destination
            .players
            .load()
            .iter()
            .any(|p| Arc::ptr_eq(p, &viewer.player))
    );
    assert!(
        !source
            .players
            .load()
            .iter()
            .any(|p| Arc::ptr_eq(p, &viewer.player))
    );
    assert!(
        viewer
            .player
            .teleport_id_count
            .load(std::sync::atomic::Ordering::Relaxed)
            > teleport
    );
    assert!(!viewer.client().is_closed());
    viewer.loaded(&scenario.server);
}

pub fn events(viewer: &Session) -> Vec<(Uuid, i32)> {
    viewer
        .packets()
        .into_iter()
        .filter_map(|packet| {
            let mut bytes = packet.as_ref();
            if bytes.get_var_int().unwrap().0 != BOSS_EVENT.to_id(CURRENT_MC_VERSION) {
                return None;
            }
            Some((bytes.get_uuid().unwrap(), bytes.get_var_int().unwrap().0))
        })
        .collect()
}

pub fn operations(events: &[(Uuid, i32)], uuid: Uuid) -> Vec<i32> {
    events
        .iter()
        .filter_map(|(id, op)| (*id == uuid).then_some(*op))
        .collect()
}

struct Probe<F>(F);
impl<F: Fn(&mut PacketSentEvent) + Send + Sync + 'static> EventHandler<PacketSentEvent>
    for Probe<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.packet_id == BOSS_EVENT.0 {
                (self.0)(event);
            }
        })
    }
}

pub fn probe(scenario: &Scenario, callback: impl Fn(&mut PacketSentEvent) + Send + Sync + 'static) {
    scenario
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(Arc::new(Probe(callback)), EventPriority::Normal, true);
}
