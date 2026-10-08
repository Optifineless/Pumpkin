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
