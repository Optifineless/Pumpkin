//! Real entity/world fixtures without a server, listener or ticking loop.
use super::World;
use arc_swap::ArcSwap;
use pumpkin_data::{Block, biome::Biome, dimension::Dimension};
use pumpkin_util::{math::vector2::Vector2, world_seed::Seed};
use pumpkin_world::{
    chunk::{ChunkData, ChunkLight, format::LightContainer},
    chunk_system::chunk_state::Chunk,
    generation::{
        generator::{WorldGenerator, flat::FlatGenerator},
        proto_chunk::ProtoChunk,
    },
    level::Level,
    world_info::LevelData,
};
use std::sync::{Arc, Weak};

pub struct Fixture {
    pub world: Arc<World>,
    dir: tempfile::TempDir,
}
impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self::from_dir(dir)
    }
    fn from_dir(dir: tempfile::TempDir) -> Self {
        let level = Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            dir.path().to_path_buf(),
            0,
            Dimension::OVERWORLD,
        );
        let world = Arc::new(World::load(
            level,
            Arc::new(ArcSwap::from_pointee(LevelData::default(Seed(0)))),
            Dimension::OVERWORLD,
            crate::block::registry::default_registry(),
            Weak::new(),
        ));
        Self { world, dir }
    }
    pub async fn restart(self) -> Self {
        self.world.shutdown().await;
        Self::from_dir(self.dir)
    }
    pub fn reopen_storage(&self) -> Arc<Level> {
        Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            self.dir.path().to_path_buf(),
            0,
            Dimension::OVERWORLD,
        )
    }
    pub async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

pub fn proto(biome: &Biome, floor: &Block) -> ProtoChunk {
    let generator = WorldGenerator::Flat(Box::new(FlatGenerator::new(
        Seed(0),
        Dimension::OVERWORLD,
        Vec::new(),
        biome.registry_id.to_owned(),
    )));
    let mut chunk = ProtoChunk::new(0, 0, &generator);
    chunk.flat_biome_map.fill(biome.id);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_state(x, 63, z, floor.default_state);
        }
    }
    chunk.light = ChunkLight {
        sky_light: vec![LightContainer::new_empty(15); Dimension::OVERWORLD.height as usize / 16]
            .into_boxed_slice(),
        block_light: vec![LightContainer::new_empty(0); Dimension::OVERWORLD.height as usize / 16]
            .into_boxed_slice(),
    };
    chunk
}

pub fn publish(world: &World, proto: ProtoChunk) -> Arc<ChunkData> {
    let mut chunk = Chunk::Proto(Box::new(proto));
    chunk.upgrade_to_level_chunk(
        &Dimension::OVERWORLD,
        &pumpkin_config::lighting::LightingEngineConfig::default(),
    );
    let Chunk::Level(chunk) = chunk else {
        panic!("fixture chunk must be upgraded")
    };
    world
        .level
        .loaded_chunks
        .insert(Vector2::new(chunk.x, chunk.z), chunk.clone());
    chunk
}
