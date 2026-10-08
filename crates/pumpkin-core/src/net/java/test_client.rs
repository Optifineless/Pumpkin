use super::*;

impl JavaClient {
    pub(crate) fn without_connection(gameprofile: GameProfile) -> Self {
        let (send, recv) = tokio::sync::mpsc::unbounded_channel();
        Self {
            id: 0,
            version: AtomicCell::new(CURRENT_MC_VERSION),
            gameprofile,
            config: ArcSwap::from_pointee(PlayerConfig::default()),
            server_address: String::new(),
            connection_state: AtomicCell::new(ConnectionState::Play),
            address: SocketAddr::from(([127, 0, 0, 1], 0)),
            brand: ArcSwap::from_pointee(None),
            player: ArcSwap::from_pointee(None),
            tasks: TaskTracker::new(),
            rt_handle: tokio::runtime::Handle::current(),
            close_token: CancellationToken::new(),
            outgoing_packet_queue_send: send,
            outgoing_packet_queue_recv: Some(recv),
            pending_bytes: Arc::new(AtomicUsize::new(0)),
            network_writer: std::sync::Mutex::new(None),
            network_reader: std::sync::Mutex::new(None),
            admission_reservation: std::sync::Mutex::new(None),
            read_only: AtomicBool::new(false),
            keep_alive: std::sync::Mutex::new(session::KeepAliveState::new(Instant::now())),
            inbound_bytes: AtomicUsize::new(0),
            chat_order: chat_order::ChatOrder::new(),
            packet_sequence: AtomicI32::new(-1),
            packet_limiter: PacketRateLimiter::new(false, 0.0, 0.0),
            suspend_flushing: Arc::new(AtomicBool::new(false)),
        }
    }
}
