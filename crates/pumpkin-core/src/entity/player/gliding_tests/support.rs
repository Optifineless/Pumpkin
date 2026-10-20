use super::*;
use bytes::Bytes;
use futures::FutureExt;
use pumpkin_data::{packet::clientbound::play, tracked_data::entity as tracked};
use pumpkin_protocol::{
    ClientPacket, Packet, RawPacket, ServerPacket,
    bedrock::client::set_actor_data::{CSetActorData, entity_data_flag, entity_data_key},
    codec::{var_long::VarLong, var_uint::VarUInt, var_ulong::VarULong},
    java::client::play::{CSetEntityMetadata, Metadata},
    packet::MultiVersionJavaPacket,
    serial::{PacketRead, PacketWrite},
};

#[derive(Clone, Copy, Debug)]
pub enum Route {
    Auth,
    Legacy,
    Embedded,
    Both,
    Java,
}

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub owner: TestBedrockPlayer,
    pub java: TestPlayer,
    pub observer: TestBedrockPlayer,
    pub toggles: Arc<AtomicUsize>,
    pub extra_worlds: Vec<Arc<World>>,
    pub extra_bedrock: Vec<TestBedrockPlayer>,
}

impl Fixture {
    pub async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = test_world(&server, directory.path());
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.spawn_x = 4;
            info.spawn_y = 80;
            info.spawn_z = 4;
            info.game_rules.respawn_radius = 0;
            info
        });
        server.worlds.store(Arc::new(vec![world.clone()]));
        let owner = TestBedrockPlayer::new(&world).await;
        let java = TestPlayer::new(&world);
        let observer = TestBedrockPlayer::new(&world).await;
        world.players.store(Arc::new(vec![
            owner.player.clone(),
            java.player.clone(),
            observer.player.clone(),
        ]));
        for player in [&owner.player, &java.player, &observer.player] {
            player.get_entity().set_pos(Vector3::new(4.0, 80.0, 4.0));
            player.watched_section.store(
                pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
                    Vector2::new(0, 0),
                    std::num::NonZeroU8::new(2).unwrap(),
                ),
            );
            player
                .chunk_sender
                .lock()
                .unwrap()
                .mark_sent_out_of_band(Vector2::new(0, 0));
            player
                .inventory
                .set_slot(38, ItemStack::new(1, &Item::ELYTRA));
        }
        let tracked = world
            .entity_tracker
            .get_tracked_entity(owner.player.entity_id())
            .unwrap();
        tracked.update_player(&java.player, &world);
        tracked.update_player(&observer.player, &world);
        let toggles = Arc::new(AtomicUsize::new(0));
        let calls = toggles.clone();
        server.plugin_manager.register::<EntityToggleGlideEvent, _>(
            Arc::new(Callback(move |_: &mut EntityToggleGlideEvent| {
                calls.fetch_add(1, Relaxed);
            })),
            EventPriority::Normal,
            true,
        );
        let mut f = Self {
            directory,
            server,
            world,
            owner,
            java,
            observer,
            toggles,
            extra_worlds: Vec::new(),
            extra_bedrock: Vec::new(),
        };
        f.tick(1);
        f.clear_packets();
        f
    }

    pub async fn check(check: impl AsyncFnOnce(&mut Self)) {
        let mut fixture = Self::new().await;
        let result = std::panic::AssertUnwindSafe(check(&mut fixture))
            .catch_unwind()
            .await;
        fixture.close().await;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    pub fn player(&self, route: Route) -> &Arc<Player> {
        if matches!(route, Route::Java) {
            &self.java.player
        } else {
            &self.owner.player
        }
    }

    pub fn request(&self, route: Route, gliding: bool) {
        request(route, self.player(route), &self.server, gliding);
    }

    pub fn tick(&self, count: usize) {
        for _ in 0..count {
            self.owner.player.tick(&self.server);
            self.java.player.tick(&self.server);
            self.world.entity_tracker.update_all(&self.world);
        }
    }

    pub fn clear_packets(&mut self) {
        self.owner.take_packets();
        self.java.take_packets();
        self.observer.take_packets();
    }

    pub fn assert_metadata(&mut self, route: Route, gliding: bool) {
        let id = self.player(route).entity_id();
        let flags = self.player(route).get_entity().flags.load(Relaxed);
        assert_eq!(
            flags & (1 << 7) != 0,
            gliding,
            "authoritative glide metadata"
        );
        let mut expected = Vec::new();
        Metadata::new(tracked::DATA_SHARED_FLAGS_ID, flags)
            .write(&mut expected, &CURRENT_MC_VERSION)
            .unwrap();
        assert!(
            self.java.take_packets().iter().any(|packet| {
                let mut data = packet.as_ref();
                if VarInt::decode(&mut data).unwrap().0 != play::SET_ENTITY_DATA.0 {
                    return false;
                }
                let packet = CSetEntityMetadata::read(&mut data, &CURRENT_MC_VERSION).unwrap();
                packet.entity_id.0 == id
                    && packet
                        .metadata
                        .windows(expected.len())
                        .any(|w| w == expected)
            }),
            "Java observer missing glide metadata for {route:?}"
        );
        for packets in [self.owner.take_packets(), self.observer.take_packets()] {
            assert!(
                packets
                    .iter()
                    .any(|packet| bedrock_glide_flag(packet, id) == Some(gliding)),
                "Bedrock owner/observer missing glide metadata for {route:?}"
            );
        }
    }

    pub async fn close(self) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            self.owner.close().await;
            self.observer.close().await;
            for player in &self.extra_bedrock {
                player.close().await;
            }
            for world in &self.extra_worlds {
                world.level.shutdown().await.unwrap();
            }
            self.world.level.shutdown().await.unwrap();
        })
        .await
        .unwrap();
    }
}

pub fn test_world(server: &Arc<Server>, path: &std::path::Path) -> Arc<World> {
    use pumpkin_world::world_info::{GeneratorSettings, WorldGenSettings};
    // Select flat before construction: the default noise constructor calculates strongholds.
    let mut settings =
        WorldGenSettings::from_preset_name("flat", pumpkin_util::world_seed::Seed(0)).unwrap();
    settings
        .dimensions
        .get_mut("minecraft:overworld")
        .unwrap()
        .generator
        .settings = Some(GeneratorSettings::Compound(serde_json::json!({
        "biome": "minecraft:plains", "layers": [], "structure_overrides": []
    })));
    pumpkin_world::world_info::data_files::write_world_gen_settings(
        path,
        &settings,
        pumpkin_world::world_info::MAXIMUM_SUPPORTED_WORLD_DATA_VERSION,
    )
    .unwrap();
    let world = combat_test_support::world(server, path);
    combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
    world
}

pub fn request(route: Route, player: &Arc<Player>, server: &Arc<Server>, gliding: bool) {
    if matches!(route, Route::Java) {
        assert!(gliding);
        let command = SPlayerCommand {
            entity_id: player.entity_id().into(),
            action: Action::StartFlyingElytra,
            jump_boost: VarInt(0),
        };
        let mut payload = Vec::new();
        command
            .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
            .unwrap();
        let ClientPlatform::Java(client) = player.client.as_ref() else {
            panic!("Java route")
        };
        client
            .handle_play_packet(
                player,
                server,
                &RawPacket {
                    id: SPlayerCommand::to_id(CURRENT_MC_VERSION),
                    payload: payload.into(),
                },
            )
            .unwrap();
        return;
    }
    let action = if gliding {
        PlayerActionType::StartGliding
    } else {
        PlayerActionType::StopGliding
    };
    let mut payload = Vec::new();
    let id = if matches!(route, Route::Legacy) {
        VarULong(player.entity_id() as u64)
            .write(&mut payload)
            .unwrap();
        VarInt(action as i32).write(&mut payload).unwrap();
        BlockPos::ZERO.write(&mut payload).unwrap();
        BlockPos::ZERO.write(&mut payload).unwrap();
        VarInt(0).write(&mut payload).unwrap();
        SPlayerAction::PACKET_ID
    } else {
        write_auth(player, route, action, &mut payload);
        SPlayerAuthInput::PACKET_ID
    };
    let ClientPlatform::Bedrock(client) = player.client.as_ref() else {
        panic!("Bedrock route")
    };
    client
        .handle_play_packet(
            player,
            server,
            &RawPacket {
                id,
                payload: Bytes::from(payload),
            },
        )
        .unwrap();
}

fn write_auth(player: &Player, route: Route, action: PlayerActionType, payload: &mut Vec<u8>) {
    let entity = player.get_entity();
    entity.pitch.load().write(payload).unwrap();
    entity.yaw.load().write(payload).unwrap();
    let pos = player.position();
    Vector3::new(
        pos.x as f32,
        pos.y as f32 + entity.entity_type.eye_height,
        pos.z as f32,
    )
    .write(payload)
    .unwrap();
    Vector2::new(0.0f32, 0.0).write(payload).unwrap();
    entity.yaw.load().write(payload).unwrap();
    let mut flags = Vec::new();
    if !matches!(route, Route::Embedded) {
        flags.push(if matches!(action, PlayerActionType::StartGliding) {
            InputData::StartGliding
        } else {
            InputData::StopGliding
        } as i32);
    }
    if entity.on_ground.load(Relaxed) {
        flags.push(InputData::VerticalCollision as i32);
    }
    VarUInt(flags.len() as u32).write(payload).unwrap();
    for flag in flags {
        VarInt(flag).write(payload).unwrap();
    }
    VarUInt(1).write(payload).unwrap(); // mouse input
    VarUInt(0).write(payload).unwrap();
    VarInt(0).write(payload).unwrap();
    0.0f32.write(payload).unwrap();
    0.0f32.write(payload).unwrap();
    VarULong(1).write(payload).unwrap();
    Vector3::new(0.0f32, -0.1, 0.0).write(payload).unwrap();
    false.write(payload).unwrap();
    false.write(payload).unwrap();
    let embedded = matches!(route, Route::Embedded | Route::Both);
    embedded.write(payload).unwrap();
    if embedded {
        VarUInt(1).write(payload).unwrap();
        VarInt(action as i32).write(payload).unwrap();
        BlockPos::ZERO.write(payload).unwrap();
        VarInt(0).write(payload).unwrap();
    }
    false.write(payload).unwrap();
    false.write(payload).unwrap();
    Vector2::new(0.0f32, 0.0).write(payload).unwrap();
    Vector3::new(0.0f32, 0.0, 0.0).write(payload).unwrap();
    Vector2::new(0.0f32, 0.0).write(payload).unwrap();
}

fn bedrock_glide_flag(packet: &[u8], id: i32) -> Option<bool> {
    // Offline fixtures use uncompressed batches; decode the real frame before actor data.
    let batch = packet.strip_prefix(&[pumpkin_protocol::bedrock::BEDROCK_GAME_PACKET])?;
    let packet = pumpkin_protocol::bedrock::packet_decoder::BedrockBatchDecoder::new()
        .get_game_packet(&mut std::io::Cursor::new(batch.to_vec()))
        .ok()?;
    let data = &mut packet.payload.as_ref();
    if packet.id != CSetActorData::PACKET_ID || VarULong::read(data).ok()?.0 != id as u64 {
        return None;
    }
    for _ in 0..VarUInt::read(data).ok()?.0 {
        let key = VarUInt::read(data).ok()?.0;
        let ty = VarUInt::read(data).ok()?.0;
        u8::read(data).ok()?;
        match ty {
            7 => {
                let flags = VarLong::read(data).ok()?.0;
                if key == entity_data_key::FLAGS {
                    return Some(flags & (1 << entity_data_flag::GLIDING) != 0);
                }
            }
            0 => {
                u8::read(data).ok()?;
            }
            2 => {
                VarInt::read(data).ok()?;
            }
            3 => {
                f32::read(data).ok()?;
            }
            8 => {
                Vector3::<f32>::read(data).ok()?;
            }
            _ => return None,
        }
    }
    None
}

pub struct Callback<F>(pub F);
impl<F: Fn(&mut EntityToggleGlideEvent) + Send + Sync> EventHandler<EntityToggleGlideEvent>
    for Callback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityToggleGlideEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}
