//! The `LevelAccessor` used by spawn checks, including an unpublished generation region.

use std::ops::Deref;

use pumpkin_data::{Block, BlockState, chunk::Biome, fluid::Fluid, tag::Taggable};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};
use pumpkin_util::random::{RandomGenerator, RandomImpl, get_seed, xoroshiro128::Xoroshiro};
use pumpkin_world::generation::proto_chunk::GenerationCache;

use super::World;

pub struct SpawnView<'a> {
    pub world: &'a World,
    pub cache: Option<&'a dyn GenerationCache>,
}

impl Deref for SpawnView<'_> {
    type Target = World;
    fn deref(&self) -> &World {
        self.world
    }
}

impl<'a> SpawnView<'a> {
    pub const fn live(world: &'a World) -> Self {
        Self { world, cache: None }
    }
    pub const fn generation(world: &'a World, cache: &'a dyn GenerationCache) -> Self {
        Self {
            world,
            cache: Some(cache),
        }
    }

    #[must_use]
    pub fn get_block_state(&self, pos: &BlockPos) -> &'static BlockState {
        self.cache.map_or_else(
            || self.world.get_block_state(pos),
            |cache| GenerationCache::get_block_state(cache, &pos.0).to_state(),
        )
    }
    #[must_use]
    pub fn get_block(&self, pos: &BlockPos) -> &'static Block {
        Block::from_state_id(self.get_block_state(pos).id)
    }
    #[must_use]
    pub fn get_fluid(&self, pos: &BlockPos) -> &'static Fluid {
        Fluid::from_state_id(self.get_block_state(pos).id).unwrap_or(&Fluid::EMPTY)
    }
    #[must_use]
    pub fn get_biome(&self, pos: &BlockPos) -> &'static Biome {
        self.cache.map_or_else(
            || self.world.get_biome(pos),
            |cache| cache.get_biome_for_terrain_gen(pos.0.x, pos.0.y, pos.0.z),
        )
    }
    #[must_use]
    pub fn moon_brightness(&self) -> f32 {
        // WorldGenRegion builds only static environment layers; MOON_PHASE defaults to FULL_MOON
        // (EnvironmentAttributes.java:70-71). Runtime uses its timeline.
        let phase = if self.cache.is_some() {
            pumpkin_data::environment_attribute::MoonPhase::FullMoon
        } else {
            self.world.get_moon_phase()
        };
        crate::entity::mob::equipment::moon_brightness(i64::from(phase.index()) * 24000)
    }

    #[must_use]
    pub fn has_structure_piece(&self, pos: &BlockPos, names: &[&str]) -> bool {
        use pumpkin_world::generation::spawn_structures::structure_bounds_from_data;
        let contains =
            |bounds: &pumpkin_world::generation::spawn_structures::StructureSpawnBounds| {
                names.iter().any(|name| {
                    pumpkin_data::structures::StructureKeys::from_name(name)
                        == Some(bounds.structure)
                }) && bounds.pieces.iter().any(|piece| piece.contains_pos(&pos.0))
            };
        if let Some(cache) = self.cache {
            return cache
                .get_chunk(pos.0.x >> 4, pos.0.z >> 4)
                .is_some_and(|chunk| chunk.spawn_structures.iter().any(contains));
        }
        self.world
            .level
            .read_chunk_sync(&pos.chunk_position(), |chunk| {
                structure_bounds_from_data(
                    chunk
                        .get_custom_data("murgicraft", "spawn_structures")
                        .as_ref(),
                )
                .iter()
                .any(contains)
            })
            .unwrap_or(false)
    }
    fn light(&self, pos: &BlockPos, sky: bool) -> u8 {
        let Some(cache) = self.cache else {
            return if sky {
                self.world.get_sky_light_level(pos)
            } else {
                self.world.get_block_light_level(pos).unwrap_or(0)
            };
        };
        cache.get_brightness(&pos.0, sky)
    }
    #[must_use]
    pub fn get_block_light_level(&self, pos: &BlockPos) -> u8 {
        self.light(pos, false)
    }
    #[must_use]
    pub fn get_raw_brightness(&self, pos: &BlockPos, darken: u8) -> u8 {
        self.light(pos, true)
            .saturating_sub(darken)
            .max(self.light(pos, false))
    }
    #[must_use]
    pub fn get_max_local_raw_brightness(&self, pos: &BlockPos) -> u8 {
        // LevelReader.getMaxLocalRawBrightness uses the accessor's sky darkening.
        self.get_raw_brightness(pos, self.sky_darken())
    }
    #[must_use]
    pub fn sky_darken(&self) -> u8 {
        // WorldGenRegion.getSkyDarken is always zero, including at night.
        self.cache
            .map_or_else(|| self.world.spawn_sky_darken(), |_| 0)
    }
    pub fn difficulty_at(
        &self,
        pos: pumpkin_util::math::vector3::Vector3<f64>,
    ) -> crate::entity::mob::equipment::RegionalDifficulty {
        use crate::entity::mob::equipment::{RegionalDifficulty, moon_brightness};
        if self.cache.is_none() {
            return RegionalDifficulty::at(self.world, pos);
        }
        // WorldGenRegion.getCurrentDifficultyAt uses zero inhabited time.
        let time = self
            .world
            .level_time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .time_of_day;
        RegionalDifficulty::calculate(
            self.world.level_info.load().difficulty,
            time,
            0,
            moon_brightness(time),
        )
    }
    #[must_use]
    pub fn get_world_surface_height(&self, x: i32, z: i32) -> i32 {
        self.cache.map_or_else(
            || {
                self.world.get_heightmap_height(
                    pumpkin_world::chunk::ChunkHeightmapType::WorldSurface,
                    x,
                    z,
                ) + 1
            },
            |cache| cache.get_top_y(&pumpkin_util::HeightMap::WorldSurface, x, z),
        )
    }
    #[must_use]
    pub fn can_see_sky(&self, pos: &BlockPos) -> bool {
        self.light(pos, true) == 15
    }
    #[must_use]
    pub fn pathfinding_cost_from_light(&self, pos: &BlockPos) -> f32 {
        // LevelReader.getPathfindingCostFromLightLevels uses local raw brightness after sky darkening.
        super::brightness::light_level_curve(
            self.get_max_local_raw_brightness(pos),
            self.dimension.ambient_light,
        ) - 0.5
    }
    #[must_use]
    pub fn is_bright_enough_to_spawn(&self, pos: &BlockPos) -> bool {
        self.get_raw_brightness(pos, 0) > 8
    }
    #[must_use]
    pub fn check_animal_spawn_rules(&self, pos: &BlockPos) -> bool {
        self.get_block(&pos.down())
            .has_tag(&pumpkin_data::tag::Block::MINECRAFT_ANIMALS_SPAWNABLE_ON)
            && self.is_bright_enough_to_spawn(pos)
    }
    #[must_use]
    pub fn is_dark_enough_to_spawn(&self, pos: &BlockPos, thunder: bool) -> bool {
        monster_is_dark(
            self,
            pos,
            thunder,
            &mut RandomGenerator::Xoroshiro(Xoroshiro::from_seed(get_seed())),
        )
    }
    #[must_use]
    pub fn check_surface_water_animal_spawn_rules(&self, pos: &BlockPos) -> bool {
        pos.0.y >= self.sea_level - 13
            && pos.0.y <= self.sea_level
            && self
                .get_fluid(&pos.down())
                .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
            && self.get_block(&pos.up()) == &Block::WATER
    }
    #[must_use]
    pub fn check_surface_ageable_water_creature_spawn_rules(&self, pos: &BlockPos) -> bool {
        self.check_surface_water_animal_spawn_rules(pos)
    }
    #[must_use]
    pub fn is_space_empty(&self, bounds: BoundingBox) -> bool {
        BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos()).all(|pos| {
            !World::check_collision(&bounds, pos, self.get_block_state(&pos), false, |_| ())
        })
    }
    #[must_use]
    pub fn contains_any_liquid(&self, bounds: BoundingBox) -> bool {
        let min = bounds.min_block_pos();
        let max = BlockPos::new(
            bounds.max.x.ceil() as i32 - 1,
            bounds.max.y.ceil() as i32 - 1,
            bounds.max.z.ceil() as i32 - 1,
        );
        BlockPos::iterate(min, max).any(|pos| self.get_fluid(&pos) != &Fluid::EMPTY)
    }
}

// Monster.isDarkEnoughToSpawn reads its light source here, including the source's sky darkening.
pub(super) trait SpawnLightSource {
    fn sky(&self, pos: &BlockPos) -> u8;
    fn block(&self, pos: &BlockPos) -> u8;
    fn darken(&self) -> u8;
    fn dimension(&self) -> &pumpkin_data::dimension::Dimension;
}
impl SpawnLightSource for SpawnView<'_> {
    fn sky(&self, pos: &BlockPos) -> u8 {
        self.light(pos, true)
    }
    fn block(&self, pos: &BlockPos) -> u8 {
        self.light(pos, false)
    }
    fn darken(&self) -> u8 {
        self.sky_darken()
    }
    fn dimension(&self) -> &pumpkin_data::dimension::Dimension {
        &self.world.dimension
    }
}
fn monster_is_dark(
    source: &impl SpawnLightSource,
    pos: &BlockPos,
    thunder: bool,
    rng: &mut RandomGenerator,
) -> bool {
    let sky = source.sky(pos);
    if i32::from(sky) > rng.next_bounded_i32(32) {
        return false;
    }
    let block = source.block(pos);
    let dimension = source.dimension();
    if dimension.monster_spawn_block_light_limit < 15
        && block > dimension.monster_spawn_block_light_limit
    {
        return false;
    }
    let brightness =
        crate::entity::mob::spawn::monster_spawn_brightness(sky, block, source.darken(), thunder);
    i32::from(brightness) <= dimension.monster_spawn_light_level.get(rng)
}
