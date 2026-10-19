use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use bytes::Bytes;
use pumpkin_config::chunk::AnvilChunkConfig;
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::NoteBlockLikeProperties,
    data_component_impl::NoteBlockSoundImpl,
    item::Item,
    item_stack::ItemStack,
    packet::clientbound::play::{BLOCK_EVENT, SOUND},
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, SUseItemOn},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};
use pumpkin_world::{
    chunk::{
        ChunkData,
        format::anvil::AnvilChunkFile,
        io::{FileIO, LoadedData, file_manager::ChunkFileManager},
    },
    world::BlockFlags,
};
use std::{sync::Arc, time::Duration};

pub(super) const NOTE: BlockPos = BlockPos::new(8, 64, 8);
pub(super) const CUSTOM_SOUND: &str = "murgicraft:heads.test_sound";
const BOUND: Duration = Duration::from_secs(30);

pub(super) struct Fixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        world
            .active_chunks
            .write()
            .unwrap()
            .insert(NOTE.chunk_position());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let player = TestPlayer::new(&world);
        player.player.permission_lvl.store(PermissionLvl::Four);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 11.5));
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    pub fn put(&self, position: BlockPos, block: &Block) {
        self.world
            .set_block_state(&position, block.default_state.id, BlockFlags::NOTIFY_ALL);
    }

    pub fn prepare(&mut self, support: &Block) {
        self.put(BlockPos::new(9, 64, 8), &Block::AIR);
        self.put(NOTE.up(), &Block::AIR);
        self.put(NOTE, &Block::AIR);
        self.put(NOTE.down(), support);
        self.use_stack_at(ItemStack::new(1, &Item::NOTE_BLOCK), NOTE.down());
        assert_eq!(self.world.get_block(&NOTE), &Block::NOTE_BLOCK);
        self.set_note(24);
        self.world.flush_synced_block_events();
        self.player.take_packets();
    }

    pub fn set_note(&self, value: u8) {
        let mut props =
            NoteBlockLikeProperties::from_state_id(self.world.get_block_state_id(&NOTE));
        props.note = value;
        self.world.set_block_state(
            &NOTE,
            props.to_state_id(&Block::NOTE_BLOCK),
            BlockFlags::FORCE_STATE,
        );
    }

    pub fn use_stack(&self, stack: ItemStack) {
        self.use_stack_at(stack, NOTE);
    }

    fn use_stack_at(&self, stack: ItemStack, clicked_pos: BlockPos) {
        self.player.player.inventory().set_stack(0, stack);
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: clicked_pos,
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    pub fn place_head(&self, item: &'static Item, sound: Option<&str>) {
        let mut stack = ItemStack::new(1, item);
        if let Some(sound) = sound {
            // Literal component input, independent of the block entity's NBT loader.
            stack.set_data_component(
                NoteBlockSoundImpl::read_data(&NbtTag::String(sound.into())).unwrap(),
            );
        }
        self.use_stack(stack);
        assert_eq!(
            self.world.get_block(&NOTE.up()).item_id,
            item.id,
            "real Java item placement failed"
        );
    }

    pub fn attack(&mut self) -> Vec<Bytes> {
        self.player.take_packets();
        self.player.client().handle_player_action(
            &self.player.player,
            &SPlayerAction {
                status: VarInt(0),
                position: NOTE,
                face: 1,
                sequence: VarInt(2),
            },
            &self.server,
        );
        self.world.flush_synced_block_events();
        self.player.take_packets()
    }

    pub fn power(&mut self) -> Vec<Bytes> {
        self.player.take_packets();
        self.put(BlockPos::new(9, 64, 8), &Block::REDSTONE_BLOCK);
        self.world.flush_synced_block_events();
        self.player.take_packets()
    }

    pub async fn reload(&self) {
        tokio::time::timeout(BOUND, async {
            self.world.save().await;
            assert!(self.world.unload_block_entities(NOTE.chunk_position()));
            assert!(
                !self
                    .world
                    .block_entities
                    .contains_key(&NOTE.chunk_position())
            );
            let terrain = self
                .world
                .level
                .loaded_chunks
                .get(&NOTE.chunk_position())
                .unwrap()
                .clone();
            self.world
                .level
                .write_chunks(vec![(NOTE.chunk_position(), terrain)])
                .await;
            self.world
                .level
                .chunk_saver
                .block_and_await_ongoing_tasks()
                .await
                .unwrap();
            self.world
                .level
                .loaded_chunks
                .remove(&NOTE.chunk_position());
            let disk = ChunkFileManager::<AnvilChunkFile<ChunkData>>::in_dimension(
                AnvilChunkConfig::default(),
                self.world.dimension.clone(),
            );
            let (send, mut receive) = tokio::sync::mpsc::channel(1);
            disk.fetch_chunks(
                &self.world.level.level_folder,
                &[NOTE.chunk_position()],
                send,
            )
            .await;
            let chunk = match receive.recv().await {
                Some(LoadedData::Loaded(chunk)) => chunk,
                Some(LoadedData::Error((_, error))) => {
                    panic!("note fixture disk reload failed: {error}")
                }
                Some(LoadedData::Missing(_)) | None => {
                    panic!("note fixture disk reload missing terrain")
                }
            };
            self.world
                .level
                .loaded_chunks
                .insert(NOTE.chunk_position(), chunk);
        })
        .await
        .expect("note fixture save/unload/reload exceeded its bound");
    }

    pub async fn finish(self) {
        tokio::time::timeout(BOUND, self.world.shutdown())
            .await
            .unwrap();
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum EventSound {
    Named(String),
    Registered(u16),
}

#[derive(Debug)]
pub(super) struct SoundPacket {
    pub sound: EventSound,
    pub category: i32,
    pub position: Vector3<i32>,
    pub volume: f32,
    pub pitch: f32,
}

pub(super) fn sounds(packets: &[Bytes]) -> Vec<SoundPacket> {
    packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if data.get_var_int().unwrap().0 != SOUND.0 {
                return None;
            }
            let holder = data.get_var_int().unwrap().0;
            let sound = if holder == 0 {
                let name = data.get_str().unwrap().to_string();
                assert!(
                    !data.get_bool().unwrap(),
                    "custom event must retain variable range"
                );
                EventSound::Named(name)
            } else {
                EventSound::Registered(u16::try_from(holder - 1).unwrap())
            };
            let result = SoundPacket {
                sound,
                category: data.get_var_int().unwrap().0,
                position: Vector3::new(
                    data.get_i32_be().unwrap(),
                    data.get_i32_be().unwrap(),
                    data.get_i32_be().unwrap(),
                ),
                volume: data.get_f32_be().unwrap(),
                pitch: data.get_f32_be().unwrap(),
            };
            data.get_i64_be().unwrap();
            assert!(data.is_empty());
            Some(result)
        })
        .collect()
}

pub(super) fn block_events(packets: &[Bytes]) -> usize {
    packets
        .iter()
        .filter(|packet| packet.as_ref().get_var_int().unwrap().0 == BLOCK_EVENT.0)
        .count()
}

pub(super) fn assert_sound(packets: &[Bytes], expected: &EventSound, pitch: f32) {
    let decoded = sounds(packets);
    assert_eq!(
        decoded.len(),
        1,
        "expected exactly one note event sound: {decoded:?}"
    );
    let actual = &decoded[0];
    assert_eq!(&actual.sound, expected);
    assert_eq!(actual.category, 2, "vanilla RECORDS sound category");
    assert_eq!(
        actual.position,
        Vector3::new(68, 516, 68),
        "note sound must use block center"
    );
    assert_eq!(actual.volume, 3.0);
    assert_eq!(actual.pitch, pitch);
    assert_eq!(
        block_events(packets),
        1,
        "successful note event must reach client"
    );
}
