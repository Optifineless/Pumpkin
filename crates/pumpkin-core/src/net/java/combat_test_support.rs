pub use super::outgoing::{Completion, OutgoingPacket};
#[allow(
    clippy::wildcard_imports,
    reason = "Keep the shared baseline namespace for scenario and host fixtures"
)]
use super::*;
use crate::entity::EntityBase;
use crate::net::ClientPlatform;
use crate::world::World;
use std::sync::atomic::AtomicU64;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

pub struct TestPlayer {
    pub player: Arc<Player>,
    packets: UnboundedReceiver<OutgoingPacket>,
}

impl TestPlayer {
    #[must_use]
    pub fn new(world: &Arc<World>) -> Self {
        let gameprofile = GameProfile {
            id: uuid::Uuid::new_v4(),
            name: "combat-test".into(),
            properties: ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let profile = gameprofile.clone();
        let (client, recv) = captured_client(gameprofile);
        let player = Arc::new(Player::new(
            Arc::new(ClientPlatform::Java(client)),
            profile,
            PlayerConfig::default(),
            world,
            pumpkin_util::GameMode::Survival,
        ));
        player.set_client_loaded(true);
        player
            .watched_section
            .store(pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
                player.get_entity().chunk_pos.load(),
                std::num::NonZeroU8::new(2).unwrap(),
            ));
        world.players.store(Arc::new(vec![player.clone()]));
        world.entity_tracker.add_entity(
            &(player.clone() as Arc<dyn crate::entity::EntityBase>),
            world,
        );
        Self {
            player,
            packets: recv,
        }
    }

    #[must_use]
    pub fn client(&self) -> &JavaClient {
        match self.player.client.as_ref() {
            ClientPlatform::Java(client) => client,
            ClientPlatform::Bedrock(_) => panic!("Java combat fixture required"),
        }
    }

    /// Completes packet write barriers while exercising an async player operation.
    pub async fn with_outgoing_writer<F: std::future::Future>(
        &mut self,
        operation: F,
    ) -> F::Output {
        self.with_outgoing_writer_inner(operation, None).await
    }

    pub async fn with_outgoing_writer_and_capture<F: std::future::Future>(
        &mut self,
        operation: F,
        packets: &mut Vec<bytes::Bytes>,
    ) -> F::Output {
        self.with_outgoing_writer_inner(operation, Some(packets))
            .await
    }

    async fn with_outgoing_writer_inner<F: std::future::Future>(
        &mut self,
        operation: F,
        mut capture: Option<&mut Vec<bytes::Bytes>>,
    ) -> F::Output {
        tokio::pin!(operation);
        loop {
            tokio::select! {
                result = &mut operation => return result,
                Some(packet) = self.packets.recv() => {
                    if let OutgoingPacket::Data { data, completion } = packet {
                        decrement_pending_bytes(&self.client().pending_bytes, data.len());
                        if let Some(packets) = capture.as_mut() {
                            packets.push(data);
                        }
                        if let Some(outgoing::Completion::Framed(done) | outgoing::Completion::Flushed(done)) = completion {
                            let _ = done.send(());
                        }
                    }
                }
            }
        }
    }

    /// Waits for the next data packet enqueued by a concurrent server operation.
    pub async fn receive_packet(&mut self) -> Option<bytes::Bytes> {
        while let Some(packet) = self.packets.recv().await {
            if let OutgoingPacket::Data { data, .. } = packet {
                return Some(data);
            }
        }
        None
    }

    pub fn take_packets(&mut self) -> Vec<bytes::Bytes> {
        let mut packets = Vec::new();
        while let Ok(packet) = self.packets.try_recv() {
            if let OutgoingPacket::Data { data, .. } = packet {
                packets.push(data);
            }
        }
        packets
    }
}

/// Creates the same Java transport fixture before the real server login constructs a player.
pub fn captured_client(profile: GameProfile) -> (JavaClient, UnboundedReceiver<OutgoingPacket>) {
    let config = PlayerConfig::default();
    let (send, recv) = tokio::sync::mpsc::unbounded_channel();
    let client = JavaClient {
        id: NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed),
        gameprofile: profile,
        config: ArcSwap::from_pointee(config),
        server_address: String::new(),
        address: "127.0.0.1:0".parse().unwrap(),
        connection_state: AtomicCell::new(ConnectionState::Play),
        close_token: tokio_util::sync::CancellationToken::new(),
        tasks: TaskTracker::new(),
        rt_handle: tokio::runtime::Handle::current(),
        outgoing_packet_queue_send: send,
        outgoing_packet_queue_recv: None,
        pending_bytes: Arc::new(AtomicUsize::new(0)),
        version: AtomicCell::new(pumpkin_data::packet::CURRENT_MC_VERSION),
        network_writer: std::sync::Mutex::new(None),
        network_reader: std::sync::Mutex::new(None),
        brand: ArcSwap::from_pointee(None),
        player: ArcSwap::from_pointee(None),
        admission_reservation: std::sync::Mutex::new(None),
        read_only: AtomicBool::new(false),
        keep_alive: std::sync::Mutex::new(session::KeepAliveState::new(Instant::now())),
        inbound_bytes: AtomicUsize::new(0),
        chat_order: chat_order::ChatOrder::new(),
        packet_sequence: AtomicI32::new(-1),
        packet_limiter: PacketRateLimiter::new(false, 0.0, 0.0),
        suspend_flushing: Arc::new(AtomicBool::new(false)),
    };
    (client, recv)
}

/// Counts tracked connection tasks, including the persistent chat worker.
pub fn client_task_count(client: &JavaClient) -> usize {
    client.tasks.len()
}

/// Reads the player currently published by the connection owner.
pub fn published_player(client: &JavaClient) -> Option<Arc<Player>> {
    (*client.player.load_full()).clone()
}
