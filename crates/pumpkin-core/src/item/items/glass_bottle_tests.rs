use super::*;
use crate::{
    entity::{Entity, EntityBase, area_effect_cloud::AreaEffectCloudEntity, living::LivingEntity},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::spawn_test_support,
};
use pumpkin_data::{
    Block, BlockStateId, biome::Biome, entity::EntityType, statistic::StatisticCategory,
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem};
use pumpkin_util::{GameMode, Hand};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

const WATER: BlockPos = BlockPos::new(8, 65, 11);
const OBSTACLE: BlockPos = BlockPos::new(8, 65, 10);

#[path = "glass_bottle_review_tests.rs"]
mod review;

struct BottleFixture {
    _dir: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    user: TestPlayer,
}

impl BottleFixture {
    fn new() -> Self {
        Self::configured(|_| {})
    }

    fn configured(configure: impl FnOnce(&mut Server)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(dir.path());
        configure(Arc::get_mut(&mut server).unwrap());
        let world = combat_test_support::world(&server, dir.path());
        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        chunk.set_block_state(8, 65, 11, Block::WATER.default_state);
        spawn_test_support::publish(&world, chunk);
        let user = TestPlayer::new(&world);
        user.player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        Self {
            _dir: dir,
            server,
            world,
            user,
        }
    }

    fn put(&self, pos: BlockPos, state: BlockStateId) {
        self.world
            .set_block_state(&pos, state, BlockFlags::NOTIFY_ALL);
        assert_eq!(self.world.get_block_state_id(&pos), state);
    }

    fn hold_bottle(&self, hand: Hand) {
        self.user
            .player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(1, &Item::GLASS_BOTTLE));
    }

    fn use_bottle(&self, hand: Hand) {
        self.user.client().handle_use_item(
            &self.user.player,
            &SUseItem {
                hand: VarInt(i32::from(hand != Hand::Right)),
                sequence: VarInt(1),
                yaw: 0.0,
                pitch: 0.0,
            },
            &self.server,
        );
    }

    #[track_caller]
    fn assert_water(&self, hand: Hand) {
        let held = self.user.player.inventory().get_stack_in_hand(hand);
        assert_eq!(held.item, &Item::POTION);
        assert_eq!(held.item_count, 1);
        assert_eq!(
            held.get_data_component::<PotionContentsImpl>()
                .unwrap()
                .potion_id,
            Some(i32::from(Potion::WATER.id))
        );
    }

    #[track_caller]
    fn assert_empty_bottle(&self, hand: Hand) {
        let held = self.user.player.inventory().get_stack_in_hand(hand);
        assert_eq!(held.item, &Item::GLASS_BOTTLE);
        assert_eq!(held.item_count, 1);
    }

    fn look_from(&self, eye_y: f64, z: f64) {
        let entity = self.user.player.get_entity();
        entity.set_pos(Vector3::new(8.5, eye_y - entity.get_eye_height(), z));
    }

    async fn finish(self) {
        assert!(self.world.level.shutdown().await.is_ok());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_stops_at_a_wall_without_consumption_or_use_stat() {
    let fixture = BottleFixture::new();
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture
        .user
        .player
        .inventory()
        .set_stack_in_hand(Hand::Right, sword.clone());
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    fixture.assert_water(Hand::Left);

    fixture.put(OBSTACLE, Block::STONE.default_state.id);
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    fixture.assert_empty_bottle(Hand::Left);
    assert!(
        fixture
            .user
            .player
            .inventory()
            .held_item()
            .are_equal(&sword)
    );
    assert_eq!(
        fixture
            .user
            .player
            .stats
            .lock()
            .unwrap()
            .get(StatisticCategory::Used, i32::from(Item::GLASS_BOTTLE.id)),
        1,
        "only the visible-water control awards a successful use"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_uses_outline_shapes_and_accepts_waterlogged_hits() {
    let fixture = BottleFixture::new();
    for (slab_type, waterlogged, fills) in [
        ("bottom", "false", true),
        ("top", "false", false),
        ("top", "true", true),
    ] {
        let slab = Block::OAK_SLAB
            .from_properties(&[("type", slab_type), ("waterlogged", waterlogged)])
            .to_state_id(&Block::OAK_SLAB);
        fixture.put(OBSTACLE, slab);
        fixture.hold_bottle(Hand::Right);
        fixture.use_bottle(Hand::Right);
        if fills {
            fixture.assert_water(Hand::Right);
        } else {
            fixture.assert_empty_bottle(Hand::Right);
        }
    }

    // Above the fluid surface, the waterlogged top slab is hit only by its outline.
    fixture.look_from(65.95, 8.5);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_selects_source_fluids_and_stops_at_lava() {
    let fixture = BottleFixture::new();
    let flowing_water = Block::WATER
        .from_properties(&[("level", "1")])
        .to_state_id(&Block::WATER);
    fixture.put(WATER, flowing_water);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_empty_bottle(Hand::Right);

    fixture.put(WATER, Block::WATER.default_state.id);
    fixture.put(OBSTACLE, flowing_water);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);

    // Keep an air gap: adjacent water would turn the source lava into obsidian on placement.
    fixture.put(OBSTACLE, Block::AIR.default_state.id);
    fixture.put(WATER, Block::AIR.default_state.id);
    fixture.put(BlockPos::new(8, 65, 12), Block::WATER.default_state.id);
    fixture.put(OBSTACLE, Block::LAVA.default_state.id);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_empty_bottle(Hand::Right);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_clips_the_fluid_height_including_water_above() {
    let fixture = BottleFixture::new();
    fixture.look_from(65.95, 8.5);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_empty_bottle(Hand::Right);

    fixture.put(WATER.up(), Block::WATER.default_state.id);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_preserves_vanilla_ray_boundary_rules() {
    let fixture = BottleFixture::new();
    let behind = BlockPos::new(8, 65, 9);
    fixture.put(behind, Block::STONE.default_state.id);
    // Grid traversal includes this face, but the ray is leaving the stone behind it.
    fixture.look_from(65.62, 10.0);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);

    fixture.put(behind, Block::AIR.default_state.id);
    // AABB.clip excludes a source-water face exactly at the 4.5-block endpoint.
    fixture.look_from(65.62, 6.5);
    fixture.hold_bottle(Hand::Right);
    fixture.use_bottle(Hand::Right);
    fixture.assert_empty_bottle(Hand::Right);

    // VoxelShape.clip still accepts a ray whose interior test point is in the fluid.
    fixture.look_from(65.62, 11.5);
    fixture.use_bottle(Hand::Right);
    fixture.assert_water(Hand::Right);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_uses_the_players_block_interaction_range() {
    let fixture = BottleFixture::new();
    fixture.put(WATER, Block::AIR.default_state.id);
    fixture.put(BlockPos::new(8, 65, 13), Block::WATER.default_state.id);
    fixture.look_from(65.62, 8.1);
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    fixture.assert_empty_bottle(Hand::Left);
    assert!(!fixture.user.player.inventory().contains_item(&Item::POTION));

    fixture.user.player.gamemode.store(GameMode::Creative);
    fixture.use_bottle(Hand::Left);
    fixture.assert_empty_bottle(Hand::Left);
    assert!(fixture.user.player.inventory().contains_item(&Item::POTION));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fluids_animals_bottle_keeps_dragon_breath_ahead_of_the_water_clip() {
    let fixture = BottleFixture::new();
    fixture.put(OBSTACLE, Block::STONE.default_state.id);
    let dragon: Arc<dyn EntityBase> = Arc::new(LivingEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(12.5, 64.0, 8.5),
        &EntityType::ENDER_DRAGON,
    )));
    let cloud = AreaEffectCloudEntity::create(
        Entity::new(
            fixture.world.clone(),
            Vector3::new(8.5, 65.0, 9.5),
            &EntityType::AREA_EFFECT_CLOUD,
        ),
        ItemStack::EMPTY.clone(),
        Vec::new(),
        600,
        1.5,
        20,
        0,
        0.0,
        0,
    );
    cloud.set_owner(Some(dragon.as_ref()));
    fixture
        .world
        .entities
        .store(Arc::new(vec![dragon, cloud.clone()]));
    fixture.hold_bottle(Hand::Left);
    fixture.use_bottle(Hand::Left);
    assert_eq!(
        fixture.user.player.inventory().off_hand_item().item,
        &Item::DRAGON_BREATH
    );
    assert_eq!(cloud.radius(), 1.0);
    fixture.finish().await;
}
