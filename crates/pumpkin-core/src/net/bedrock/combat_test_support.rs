use super::*;
use crate::{
    entity::{EntityBase, player::Player},
    net::{ClientPlatform, GameProfile, PlayerConfig},
    world::World,
};
use arc_swap::ArcSwap;
use pumpkin_util::GameMode;

pub struct TestBedrockPlayer {
    pub player: Arc<Player>,
    packets: UnboundedReceiver<OutgoingPacket>,
}
impl TestBedrockPlayer {
    pub async fn new(world: &Arc<World>) -> Self {
        let address = "127.0.0.1:0".parse().unwrap();
        let session = Arc::new(NetherNetSession::offline(address));
        let client = BedrockClient::new(
            session,
            address,
            Arc::default(),
            PacketRateLimiter::new(false, 0.0, 0.0),
        );
        let packets = client
            .outgoing_packet_queue_recv
            .lock()
            .await
            .take()
            .unwrap();
        let profile = GameProfile {
            id: uuid::Uuid::new_v4(),
            name: "bedrock-test".into(),
            properties: ArcSwap::from_pointee(Vec::new()),
            profile_actions: None,
        };
        let player = Arc::new(Player::new(
            Arc::new(ClientPlatform::Bedrock(Arc::new(client))),
            profile,
            PlayerConfig::default(),
            world,
            GameMode::Survival,
        ));
        player.set_client_loaded(true);
        world.players.store(Arc::new(vec![player.clone()]));
        world
            .entity_tracker
            .add_entity(&(player.clone() as Arc<dyn EntityBase>), world);
        Self { player, packets }
    }
    pub fn client(&self) -> &BedrockClient {
        match self.player.client.as_ref() {
            ClientPlatform::Bedrock(client) => client,
            ClientPlatform::Java(_) => panic!("Bedrock fixture required"),
        }
    }
    pub fn take_packets(&mut self) -> Vec<Bytes> {
        let mut packets = Vec::new();
        while let Ok(packet) = self.packets.try_recv() {
            packets.push(packet.data);
        }
        packets
    }
    pub async fn close(&self) {
        self.client().session.close().await;
    }
}
