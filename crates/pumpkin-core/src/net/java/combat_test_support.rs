use super::*;
use crate::entity::EntityBase;
use crate::net::ClientPlatform;
use crate::world::World;

pub struct TestPlayer {
    pub player: Arc<Player>,
    packets: UnboundedReceiver<OutgoingPacket>,
}

impl TestPlayer {
    pub fn new(world: &Arc<World>) -> Self {
        crate::server::fixture_lifecycle::track_world(world);
        let gameprofile = GameProfile {
            id: uuid::Uuid::new_v4(),
            name: "combat-test".into(),
            properties: ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let profile = gameprofile.clone();
        let config = PlayerConfig::default();
        let (send, recv) = tokio::sync::mpsc::unbounded_channel();
        let client = JavaClient {
            id: 1,
            gameprofile,
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
        let player = Arc::new(Player::new(
            Arc::new(ClientPlatform::Java(client)),
            profile,
            PlayerConfig::default(),
            world,
            pumpkin_util::GameMode::Survival,
        ));
        crate::server::fixture_lifecycle::track_player(&player);
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
        tokio::pin!(operation);
        loop {
            tokio::select! {
                result = &mut operation => return result,
                Some(packet) = self.packets.recv() => {
                    if let OutgoingPacket::Data { data, completion } = packet {
                        decrement_pending_bytes(&self.client().pending_bytes, data.len());
                        if let Some(outgoing::Completion::Framed(done) | outgoing::Completion::Flushed(done)) = completion {
                            let _ = done.send(());
                        }
                    }
                }
            }
        }
    }

    pub fn take_packets(&mut self) -> Vec<bytes::Bytes> {
        let mut packets = Vec::new();
        while let Ok(packet) = self.packets.try_recv() {
            if let OutgoingPacket::Data { data, .. } = packet {
                decrement_pending_bytes(&self.client().pending_bytes, data.len());
                packets.push(data);
            }
        }
        packets
    }

    /// Captures real serialized packets and acknowledges the writer barriers while a task runs.
    pub async fn collect_packets_during(
        &mut self,
        task: impl std::future::Future<Output = ()>,
    ) -> Vec<bytes::Bytes> {
        let mut packets = Vec::new();
        tokio::pin!(task);
        loop {
            tokio::select! {
                () = &mut task => break,
                packet = self.packets.recv() => {
                    if let Some(OutgoingPacket::Data { data, completion }) = packet {
                        decrement_pending_bytes(&self.client().pending_bytes, data.len());
                        packets.push(data);
                        if let Some(super::outgoing::Completion::Framed(done)
                            | super::outgoing::Completion::Flushed(done)) = completion {
                            let _ = done.send(());
                        }
                    }
                }
            }
        }
        packets.extend(self.take_packets());
        packets
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review7_packet_collection_balances_pending_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let server = crate::server::combat_test_support::server(dir.path());
    let world = crate::server::combat_test_support::world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    fixture.take_packets();
    let player = fixture.player.clone();
    let packet = pumpkin_protocol::java::client::play::CEntityVelocity::new(
        player.entity_id().into(),
        pumpkin_util::math::vector3::Vector3::new(0.0, 0.2, 0.0),
    );
    let packets = fixture
        .collect_packets_during(async {
            let ClientPlatform::Java(client) = player.client.as_ref() else {
                panic!("Java combat fixture required");
            };
            client
                .send_packet_now_data(client.serialize_packet(&packet).unwrap())
                .await;
            player.try_send_client_packet(&packet);
        })
        .await;
    assert_eq!(packets.len(), 2);
    assert_eq!(fixture.client().pending_bytes.load(Ordering::Acquire), 0);
    crate::server::fixture_lifecycle::finish().await;
}
