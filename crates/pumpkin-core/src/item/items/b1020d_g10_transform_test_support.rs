use crate::{
    entity::{EntityBase, player::Player},
    net::{
        bedrock::combat_test_support::TestBedrockPlayer,
        java::{combat_test_support::TestPlayer, sound_test_support::decode_sounds},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{ChestLikeProperties, ChestType, HorizontalFacing},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        network_item::NetworkItemDescriptor,
        server::inventory_transaction::{
            HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
        },
    },
    codec::var_uint::VarUInt,
    java::server::play::SUseItemOn,
    ser::NetworkReadExt,
    serial::PacketRead,
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
    sync::{LockClass, Mutex},
    version::BedrockMinecraftVersion,
};
use pumpkin_world::{
    chunk::io::{FileIO, LoadedData},
    world::BlockFlags,
};
use std::sync::Arc;

pub(super) const LEFT: BlockPos = BlockPos::new(0, 64, 1);
pub(super) const RIGHT: BlockPos = BlockPos::new(1, 64, 1);

pub(super) enum TestCaptureClass {}
impl LockClass for TestCaptureClass {
    const NAME: &'static str = "transform test capture";
    const LEVEL: u8 = 40;
}

#[derive(Default)]
pub(super) struct GameEvents(pub Mutex<Vec<(String, Vector3<f64>)>, TestCaptureClass>);
impl EventHandler<GenericGameEvent> for GameEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push((event.event_key.clone(), event.position));
        Box::pin(async {})
    }
}

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub java: TestPlayer,
    pub java_observer: TestPlayer,
    pub bedrock: TestBedrockPlayer,
    pub bedrock_observer: TestBedrockPlayer,
    pub events: Arc<GameEvents>,
    _directory: tempfile::TempDir,
}
impl Fixture {
    pub async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let java = TestPlayer::new(&world);
        let java_observer = TestPlayer::new(&world);
        let bedrock = bounded(TestBedrockPlayer::new(&world)).await;
        let bedrock_observer = bounded(TestBedrockPlayer::new(&world)).await;
        let players = vec![
            java.player.clone(),
            java_observer.player.clone(),
            bedrock.player.clone(),
            bedrock_observer.player.clone(),
        ];
        for player in &players {
            player.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));
            player.get_entity().set_sneaking(true);
            player.advancements.try_lock().unwrap().player = Arc::downgrade(player);
            player.watched_section.store(
                pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
                    LEFT.chunk_position(),
                    std::num::NonZeroU8::new(2).unwrap(),
                ),
            );
        }
        for client in [bedrock.client(), bedrock_observer.client()] {
            client.version.store(BedrockMinecraftVersion::V_1_26_51);
        }
        world.players.store(Arc::new(players));
        let events = Arc::new(GameEvents::default());
        server.plugin_manager.register::<GenericGameEvent, _>(
            events.clone(),
            EventPriority::Normal,
            true,
        );
        Self {
            server,
            world,
            java,
            java_observer,
            bedrock,
            bedrock_observer,
            events,
            _directory: directory,
        }
    }
    pub fn pair(&self, left: &'static Block, right: &'static Block) {
        for (position, block, kind, count) in [
            (LEFT, left, ChestType::Left, 7),
            (RIGHT, right, ChestType::Right, 19),
        ] {
            let mut properties = ChestLikeProperties::default(block);
            properties.facing = HorizontalFacing::North;
            properties.r#type = kind;
            self.world.set_block_state(
                &position,
                properties.to_state_id(block),
                BlockFlags::FORCE_STATE,
            );
            self.world
                .get_block_entity(&position)
                .unwrap()
                .get_inventory()
                .unwrap()
                .set_stack(0, ItemStack::new(count, &Item::DIAMOND));
        }
    }
    pub async fn reload_pair(&self) {
        let chunk_pos = LEFT.chunk_position();
        bounded(self.world.save()).await;
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .unwrap()
            .clone();
        bounded(
            self.world
                .level
                .chunk_saver
                .save_chunks(&self.world.level.level_folder, vec![(chunk_pos, chunk)]),
        )
        .await
        .unwrap();
        self.world.block_entities.remove(&chunk_pos);
        self.world.level.loaded_chunks.remove(&chunk_pos);
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        bounded(self.world.level.chunk_saver.fetch_chunks(
            &self.world.level.level_folder,
            &[chunk_pos],
            send,
        ))
        .await;
        let LoadedData::Loaded(chunk) = bounded(receive.recv()).await.unwrap() else {
            panic!("saved mixed copper pair missing")
        };
        self.world.level.loaded_chunks.insert(chunk_pos, chunk);
    }
    pub fn clear(&mut self) {
        self.java.take_packets();
        self.java_observer.take_packets();
        self.bedrock.take_packets();
        self.bedrock_observer.take_packets();
        self.events.0.lock().unwrap().clear();
    }
    pub fn use_java(&self, target: BlockPos) {
        hold(&self.java.player, Hand::Right, &Item::IRON_AXE);
        java_use(&self.java, &self.server, target, Hand::Right);
    }
    pub fn use_bedrock(&self, target: BlockPos) {
        // Synthetic server hand setup: native Bedrock Survival UI does not offer an offhand axe.
        self.use_bedrock_with_hand(target, Hand::Left);
    }
    pub fn use_bedrock_with_hand(&self, target: BlockPos, hand: Hand) {
        hold(&self.bedrock.player, hand, &Item::IRON_AXE);
        let held = self.bedrock.player.inventory().get_stack_in_hand(hand);
        self.bedrock.client().handle_inventory_action(
            &self.bedrock.player,
            SInventoryTransaction {
                legacy_request_id: VarInt(0),
                legacy_set_item_slots: Vec::new(),
                has_value: true,
                actions: Vec::new(),
                transaction_type: VarUInt(2),
                transaction_data: TransactionData::UseItem(UseItemTransactionData {
                    action_type: VarInt(0),
                    trigger_type: 0,
                    block_position: target,
                    block_face: 1,
                    hot_bar_slot: VarInt(0),
                    hand: if hand == Hand::Right {
                        HandSlot::Mainhand
                    } else {
                        HandSlot::Offhand
                    },
                    item_in_hand: NetworkItemDescriptor::from(&held),
                    player_position: Vector3::new(0.5, 64.0, 0.5),
                    click_position: Vector3::new(0.5, 0.5, 0.5),
                    block_runtime_id: VarUInt(0),
                    client_prediction: 0,
                    client_cooldown_state: 0,
                }),
            },
        );
    }
    pub async fn close(self) {
        bounded(self.bedrock.close()).await;
        bounded(self.bedrock_observer.close()).await;
        bounded(self.world.level.shutdown()).await.unwrap();
    }
}

pub(super) fn hold(player: &Player, hand: Hand, item: &'static Item) {
    player
        .inventory()
        .set_stack_in_hand(hand, ItemStack::new(1, item));
}
pub(super) fn java_use(player: &TestPlayer, server: &Arc<Server>, position: BlockPos, hand: Hand) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(i32::from(hand == Hand::Left)),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            server,
        )
        .unwrap();
}

pub(super) fn java_effects(
    packets: &[bytes::Bytes],
    particle: i32,
    sound: u16,
    positions: &[BlockPos],
) {
    let sounds = decode_sounds(packets);
    assert_eq!(sounds.len(), 1, "one transform sound");
    assert_eq!(sounds[0].sound_id, sound);
    assert_eq!(sounds[0].category, 4);
    let clicked = positions[0];
    assert_eq!(
        sounds[0].position,
        Vector3::new(
            clicked.0.x * 8 + 4,
            clicked.0.y * 8 + 4,
            clicked.0.z * 8 + 4
        )
    );
    assert_eq!((sounds[0].volume, sounds[0].pitch), (1.0, 1.0));
    assert_eq!(
        java_particles(packets),
        positions
            .iter()
            .map(|position| (particle, *position, 0, false))
            .collect::<Vec<_>>()
    );
}
pub(super) fn java_particles(packets: &[bytes::Bytes]) -> Vec<(i32, BlockPos, i32, bool)> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if data.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::LEVEL_EVENT
                    .to_id(pumpkin_data::packet::CURRENT_MC_VERSION)
            {
                return None;
            }
            let result = (
                data.get_i32_be().unwrap(),
                data.get_block_pos(&pumpkin_data::packet::CURRENT_MC_VERSION)
                    .unwrap(),
                data.get_i32_be().unwrap(),
                data.get_bool().unwrap(),
            );
            assert!(data.is_empty());
            Some(result)
        })
        .collect()
}

pub(super) fn bedrock_effects(
    packets: &[bytes::Bytes],
    particle: i32,
    sound: &str,
    positions: &[BlockPos],
    clicked: BlockPos,
) {
    let mut particles = Vec::new();
    let mut sounds = Vec::new();
    for packet in packets {
        let mut data = packet.as_ref();
        assert_eq!(u8::read(&mut data).unwrap(), 0xfe);
        let size = VarUInt::read(&mut data).unwrap().0;
        assert_eq!(size as usize, data.len());
        match VarUInt::read(&mut data).unwrap().0 & 0x3ff {
            25 => {
                // Independent literal v2193 fixture: signed event, float position, signed data.
                assert_eq!(VarInt::read(&mut data).unwrap().0, particle);
                let pos = Vector3::<f32>::read(&mut data).unwrap();
                assert_eq!(VarInt::read(&mut data).unwrap().0, 0);
                assert!(data.is_empty());
                particles.push(pos);
            }
            123 => {
                let event = String::read(&mut data).unwrap();
                let pos = Vector3::<f32>::read(&mut data).unwrap();
                assert_eq!(VarInt::read(&mut data).unwrap().0, -1);
                assert_eq!(String::read(&mut data).unwrap(), "");
                assert!(!bool::read(&mut data).unwrap());
                assert!(!bool::read(&mut data).unwrap());
                assert_eq!(i64::read(&mut data).unwrap(), 0);
                assert_eq!(u8::read(&mut data).unwrap(), 0);
                assert!(data.is_empty());
                sounds.push((event, pos));
            }
            _ => {}
        }
    }
    assert_eq!(
        sounds,
        vec![(sound.to_owned(), clicked.to_centered_f64().to_f32_lossy())]
    );
    assert_eq!(
        particles,
        positions
            .iter()
            .map(|pos| pos.to_centered_f64().to_f32_lossy())
            .collect::<Vec<_>>()
    );
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), future)
        .await
        .unwrap()
}
