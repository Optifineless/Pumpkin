//! Teardown for fixtures that use real world services without running a server tick loop.
use crate::{
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
    server::Server,
    world::World,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex, Weak},
};

#[cfg(target_os = "linux")]
mod tests;

#[derive(Default)]
struct Fixtures {
    servers: Vec<Weak<Server>>,
    worlds: Vec<Arc<World>>,
    players: Vec<Weak<Player>>,
}

static FIXTURES: LazyLock<Mutex<HashMap<tokio::runtime::Id, Fixtures>>> =
    LazyLock::new(Mutex::default);

/// Tracks a configured fixture server for teardown before its runtime stops.
pub fn track_server(server: &Arc<Server>) {
    FIXTURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(tokio::runtime::Handle::current().id())
        .or_default()
        .servers
        .push(Arc::downgrade(server));
}

/// Keeps a fixture world alive until finish, including worlds absent from the server's list.
pub fn track_world(world: &Arc<World>) {
    if let Some(server) = world.server.upgrade() {
        track_server(&server);
    }
    FIXTURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(tokio::runtime::Handle::current().id())
        .or_default()
        .worlds
        .push(world.clone());
}

/// Tracks a fixture player so teardown also releases client bindings after disconnect.
pub fn track_player(player: &Arc<Player>) {
    FIXTURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(tokio::runtime::Handle::current().id())
        .or_default()
        .players
        .push(Arc::downgrade(player));
}

/// Drains fixture services and releases entity/world cycles while the test runtime is alive.
pub async fn finish() {
    let fixtures = FIXTURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&tokio::runtime::Handle::current().id())
        .unwrap_or_default();
    let servers: Vec<_> = fixtures.servers.iter().filter_map(Weak::upgrade).collect();
    let mut worlds = fixtures.worlds;
    let players: Vec<_> = fixtures.players.iter().filter_map(Weak::upgrade).collect();
    worlds.extend(players.iter().map(|player| player.world()));
    // MinecraftServer.stopServer closes connections before draining world services.
    for player in &players {
        match player.client.as_ref() {
            ClientPlatform::Java(client) => {
                client.close();
                client.await_tasks().await;
            }
            ClientPlatform::Bedrock(client) => {
                client.close().await;
                client.await_tasks().await;
            }
        }
    }
    for server in &servers {
        server.tasks.close();
        server.tasks.wait().await;
        assert!(server.player_data_storage.drain().await.is_ok());
        server.map_manager.drain().await;
        worlds.extend(server.worlds.load().iter().cloned());
    }
    let mut seen = HashSet::new();
    worlds.retain(|world| seen.insert(world.uuid));
    for world in worlds {
        // ServerChunkCache.close drains workers before releasing storage. Fixtures must
        // do this before tokio::test drops the runtime, which would abort their I/O tasks.
        assert!(world.level.shutdown().await.is_ok());
        release_entities(&world, &players);
        world.level.world_portal.store(Arc::new(None));
    }
}

fn release_entities(world: &World, players: &[Arc<Player>]) {
    let mut entities: Vec<Arc<dyn EntityBase>> = world.entities.load().iter().cloned().collect();
    entities.extend(
        world
            .players
            .load()
            .iter()
            .cloned()
            .map(|player| player as Arc<dyn EntityBase>),
    );
    entities.extend(
        world
            .entity_tracker
            .entity_map
            .iter()
            .map(|entry| entry.entity.clone()),
    );
    entities.extend(
        players
            .iter()
            .filter(|player| player.world().uuid == world.uuid)
            .cloned()
            .map(|player| player as Arc<dyn EntityBase>),
    );
    let mut seen = HashSet::new();
    while let Some(entity) = entities.pop() {
        let base = entity.get_entity();
        if !seen.insert(base.entity_id) {
            continue;
        }
        world.stop_tracking_dragon_parts(entity.as_ref());
        if let Some(player) = entity.get_player() {
            match player.client.as_ref() {
                ClientPlatform::Java(client) => client.player.store(Arc::new(None)),
                ClientPlatform::Bedrock(client) => client.player.store(Arc::new(None)),
            }
        }
        // Synthetic mounts need both directions released, even when never inserted.
        entities.extend(
            base.vehicle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        entities.extend(std::mem::take(
            &mut *base
                .passengers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ));
    }
    world.players.store(Arc::new(Vec::new()));
    world.entities.store(Arc::new(Vec::new()));
    world.entity_tracker.entity_map.clear();
}
