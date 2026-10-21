//! In-process lifecycle scenarios. No listeners, clock deadlines or automatic tick loop.
#![expect(
    clippy::unwrap_used,
    reason = "Scenario setup and assertions must fail loudly"
)]

mod entity_unload;
mod hardcore_respawn_review;
mod lifecycle;
mod movement_game_events;
mod movement_game_events_lifecycle;
mod movement_game_events_unload;
mod old_saves;
mod ownership;
mod ownership_tests;
mod persistence;
mod player_list;
mod respawn_owner_review;
pub mod riding_lock_probe;
mod transfer;
mod villager_persistence;

use super::{Server, combat_test_support};
use crate::{
    data::player_server::ServerPlayerData,
    entity::{EntityBase, player::Player},
    net::{
        ClientPlatform, GameProfile, PlayerConfig,
        java::{
            combat_test_support::{Completion, OutgoingPacket},
            combat_test_support::{captured_client, client_task_count},
        },
    },
    world::World,
};
use bytes::Bytes;
use ownership::Fixture;
pub use ownership::run;
use pumpkin_data::{Block, dimension::Dimension, entity::EntityType, packet::CURRENT_MC_VERSION};
use pumpkin_protocol::{
    ClientPacket, RawPacket, ServerPacket, java::server::play::SConfirmTeleport,
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{
    generation::generator::{WorldGenerator, flat::FlatGenerator},
    level::Level,
};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    path::Path,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

/// Bound cooperative I/O and task progress by polls, never by elapsed time.
const MAX_POLLS: usize = 4_000_000;
const MAX_OUTGOING_PER_POLL: usize = 4096;
const VIEW_RADIUS: i32 = 2;
pub const SPAWN: Vector3<f64> = Vector3::new(3.5, 64.0, 3.5);
// Each level owns scheduler threads; serialize scenarios to bound disk/worker contention.
static SCENARIO_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn drive<F: Future>(
    label: &str,
    operation: F,
    pump: impl Fn(),
    max_polls: usize,
) -> F::Output {
    tokio::pin!(operation);
    for _ in 0..max_polls {
        pump();
        let result = operation
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        if let Poll::Ready(result) = result {
            pump();
            return result;
        }
        tokio::task::yield_now().await;
    }
    panic!("scenario {label} did not complete after {max_polls} cooperative polls")
}

pub struct Session {
    pub player: Arc<Player>,
    incoming: RefCell<UnboundedReceiver<OutgoingPacket>>,
    captured: RefCell<Vec<Bytes>>,
}
impl Session {
    pub fn client(&self) -> &crate::net::java::JavaClient {
        self.player.client.java().unwrap()
    }
    /// This transport has no socket writer or skin properties; only its chat worker persists.
    pub fn is_idle(&self) -> bool {
        client_task_count(self.client()) == 1
    }
    fn pump(&self) {
        for _ in 0..MAX_OUTGOING_PER_POLL {
            let Ok(packet) = self.incoming.borrow_mut().try_recv() else {
                return;
            };
            if let OutgoingPacket::Data { data, completion } = packet {
                crate::net::decrement_pending_bytes(&self.client().pending_bytes, data.len());
                self.captured.borrow_mut().push(data);
                if let Some(Completion::Framed(done) | Completion::Flushed(done)) = completion {
                    let _ = done.send(());
                }
            }
        }
        assert!(
            self.incoming.borrow().is_empty(),
            "scenario outgoing queue exceeds {MAX_OUTGOING_PER_POLL} packets per poll"
        );
    }
    pub fn packets(&self) -> Vec<Bytes> {
        self.pump();
        std::mem::take(&mut *self.captured.borrow_mut())
    }
    pub fn packet(&self, server: &Arc<Server>, id: i32, payload: Vec<u8>) {
        let result = self.client().handle_play_packet(
            &self.player,
            server,
            &RawPacket {
                id,
                payload: payload.into(),
            },
        );
        assert!(
            result.is_ok(),
            "serverbound packet {id} rejected: {:?}",
            result.err().map(|error| error.to_string())
        );
    }
    pub fn send<'a, P: ClientPacket + ServerPacket<'a>>(&self, server: &Arc<Server>, packet: &P) {
        let mut payload = Vec::new();
        packet
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        self.packet(server, P::to_id(CURRENT_MC_VERSION), payload);
    }
    /// Acknowledge the real pending teleport through the decoder and play packet dispatcher.
    pub fn confirm_teleport(&self, server: &Arc<Server>) {
        let pending = *self.player.awaiting_teleport.lock().unwrap();
        if let Some((teleport_id, position)) = pending {
            self.send(
                server,
                &SConfirmTeleport {
                    teleport_id,
                    position,
                    yaw: self.player.get_entity().yaw.load(),
                    pitch: self.player.get_entity().pitch.load(),
                },
            );
        }
    }
    pub fn loaded(&self, server: &Arc<Server>) {
        self.confirm_teleport(server);
        self.packet(
            server,
            pumpkin_data::packet::serverbound::play::PLAYER_LOADED.to_id(CURRENT_MC_VERSION),
            Vec::new(),
        );
    }
}

#[derive(Clone)]
pub struct Scenario {
    pub server: Arc<Server>,
    sessions: Rc<RefCell<Vec<Rc<Session>>>>,
    fixture: Rc<Fixture>,
    stopped: Rc<Cell<bool>>,
}
impl std::ops::Deref for Scenario {
    type Target = Fixture;

    fn deref(&self) -> &Fixture {
        &self.fixture
    }
}
impl Scenario {
    /// Acquires the fixture permit inside `run`; use `restart` to reuse a live scenario.
    pub async fn new() -> Self {
        Self::new_with_hardcore(false).await
    }
    async fn new_with_hardcore(hardcore: bool) -> Self {
        Self::new_with_advanced(hardcore, None).await
    }
    async fn new_with_advanced(
        hardcore: bool,
        advanced: Option<pumpkin_config::AdvancedConfiguration>,
    ) -> Self {
        let isolation = SCENARIO_GATE.lock().await;
        let directory = tempfile::tempdir().unwrap();
        let server = Self::services_with_advanced(directory.path(), hardcore, advanced);
        let scenario = Self {
            server,
            sessions: Rc::default(),
            fixture: Rc::new(Fixture {
                directory,
                isolation,
            }),
            stopped: Rc::default(),
        };
        ownership::register(&scenario);
        for world in scenario.server.worlds.load().iter() {
            scenario.prepare_area(world, Vector2::new(0, 0)).await;
        }
        scenario
    }
    fn services(path: &Path, hardcore: bool) -> Arc<Server> {
        Self::services_with_advanced(path, hardcore, None)
    }
    fn services_with_advanced(
        path: &Path,
        hardcore: bool,
        advanced: Option<pumpkin_config::AdvancedConfiguration>,
    ) -> Arc<Server> {
        let mut server = combat_test_support::server(path);
        let services = Arc::get_mut(&mut server).unwrap();
        if let Some(advanced) = advanced {
            services.advanced_config = advanced;
        }
        services.basic_config.hardcore = hardcore;
        services.dimensions = vec![Dimension::OVERWORLD, Dimension::THE_NETHER];
        services.advanced_config.networking.java.view_distance =
            std::num::NonZeroU8::new(2).unwrap();
        services.advanced_config.networking.java.simulation_distance =
            std::num::NonZeroU8::new(2).unwrap();
        services.player_data_storage =
            ServerPlayerData::new(path.join("players"), Duration::from_secs(300), true);
        services.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.spawn_x = 3;
            info.spawn_y = 64;
            info.spawn_z = 3;
            info.game_rules.spawn_mobs = false;
            info.game_rules.respawn_radius = 0;
            info.game_rules.random_tick_speed = 0;
            info.game_rules.advance_weather = false;
            info
        });
        let worlds = server
            .dimensions
            .iter()
            .map(|dimension| {
                let world = Arc::new(World::load(
                    Level::from_root_folder(
                        &pumpkin_config::world::LevelConfig::default(),
                        path.to_owned(),
                        0,
                        dimension.clone(),
                    ),
                    server.level_info.clone(),
                    dimension.clone(),
                    server.block_registry.clone(),
                    Arc::downgrade(&server),
                ));
                let portal: Arc<dyn pumpkin_world::world::WorldPortalExt> =
                    Arc::new(crate::world::WorldPortal(world.clone()));
                world.level.world_portal.store(Arc::new(Some(portal)));
                world
                    .level
                    .set_world_gen(Arc::new(WorldGenerator::Flat(Box::new(
                        FlatGenerator::new(
                            world.level.seed,
                            dimension.clone(),
                            Vec::new(),
                            pumpkin_data::biome::Biome::PLAINS.registry_id.to_owned(),
                        ),
                    ))));
                world
            })
            .collect();
        server.worlds.store(Arc::new(worlds));
        server
    }
    pub async fn prepare_area(&self, world: &World, center: Vector2<i32>) {
        let mut chunks = Vec::new();
        for x in center.x - VIEW_RADIUS..=center.x + VIEW_RADIUS {
            for z in center.y - VIEW_RADIUS..=center.y + VIEW_RADIUS {
                combat_test_support::publish_empty_chunk(world, Vector2::new(x, z));
                for bx in 0..16 {
                    for bz in 0..16 {
                        world.level.set_block_state(
                            &BlockPos::new(x * 16 + bx, 63, z * 16 + bz),
                            Block::STONE.default_state.id,
                        );
                    }
                }
                let pos = Vector2::new(x, z);
                chunks.push((pos, world.level.loaded_chunks.get(&pos).unwrap().clone()));
            }
        }
        // Seed disk fixtures, then let the normal scheduler own publication and shutdown saves.
        // A raw cache insert alone has no scheduler holder and can be replaced by generation.
        let positions: Vec<_> = chunks.iter().map(|(pos, _)| *pos).collect();
        self.drive("fixture terrain save", world.level.write_chunks(chunks))
            .await;
        for pos in positions {
            world.level.loaded_chunks.remove(&pos);
        }
    }
    pub fn world(&self, dimension: &Dimension) -> Arc<World> {
        self.server.get_world_from_dimension(dimension)
    }
    pub fn entities_live(world: &World, pos: Vector2<i32>) -> bool {
        world
            .level
            .get_entity_chunk_sync(&pos)
            .is_some_and(|chunk| chunk.live.load(std::sync::atomic::Ordering::Acquire))
    }
    pub fn pump(&self) {
        for session in self.sessions.borrow().iter() {
            session.pump();
        }
    }
    pub async fn drive<F: Future>(&self, label: &str, operation: F) -> F::Output {
        drive(label, operation, || self.pump(), MAX_POLLS).await
    }
    pub async fn until(&self, label: &str, predicate: impl Fn() -> bool) {
        self.drive(label, async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    /// Tick the same graph as `Ticker`, with an explicit tick count and no clock scheduler.
    pub async fn ticks(&self, count: usize) {
        for _ in 0..count {
            let gate = self.drive("tick gate", self.server.tick_gate.lock()).await;
            self.server.tick();
            self.server.update_tick_times(0);
            drop(gate);
            self.pump();
            tokio::task::yield_now().await;
        }
    }
    /// Matches the `ReadyToPlay` branch in lib.rs after transport authentication/configuration.
    pub async fn join(&self, id: Uuid, name: &str) -> Rc<Session> {
        let profile = GameProfile {
            id,
            name: name.into(),
            properties: arc_swap::ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let (client, incoming) = captured_client(profile.clone());
        let client = Arc::new(ClientPlatform::Java(client));
        let config = PlayerConfig {
            view_distance: std::num::NonZeroU8::new(2).unwrap(),
            ..PlayerConfig::default()
        };
        let (player, world) = self
            .drive(
                "login",
                self.server.add_player(client, profile, Some(config)),
            )
            .await
            .unwrap();
        let session = Rc::new(Session {
            player,
            incoming: RefCell::new(incoming),
            captured: RefCell::new(Vec::new()),
        });
        session.client().set_player(session.player.clone());
        self.sessions.borrow_mut().push(session.clone());
        self.drive(
            "join packets",
            world.spawn_java_player(&self.server.basic_config, &session.player, &self.server),
        )
        .await;
        session.loaded(&self.server);
        self.until("join entity storage publication", || {
            session.is_idle()
                && world
                    .level
                    .get_entity_chunk_sync(&session.player.get_entity().chunk_pos.load())
                    .is_some()
        })
        .await;
        self.ticks(1).await;
        self.until("join watcher tasks", || {
            world
                .level
                .is_chunk_watched(&session.player.get_entity().chunk_pos.load())
                && Self::entities_live(&world, session.player.get_entity().chunk_pos.load())
                && session.is_idle()
        })
        .await;
        session
    }
    /// Runs production connection-owner cleanup, including UUID storage/admission release.
    pub async fn leave(&self, session: &Session) {
        self.drive(
            "connection-owner disconnect",
            crate::net::java::connection_owner::finish_disconnect(&session.player, &self.server),
        )
        .await;
    }
    pub fn command(&self, input: &str) -> i32 {
        let source = crate::command::CommandSender::Console
            .into_source(&self.server)
            .with_silent();
        self.server
            .command_dispatcher
            .load()
            .execute_input(input, &source)
            .unwrap()
    }
    pub fn mob(
        world: &Arc<World>,
        kind: &'static EntityType,
        pos: Vector3<f64>,
    ) -> Arc<dyn EntityBase> {
        let entity = crate::entity::r#type::from_type(kind, pos, world, Uuid::new_v4());
        world.add_entity_silent(entity.clone());
        entity
    }
}
