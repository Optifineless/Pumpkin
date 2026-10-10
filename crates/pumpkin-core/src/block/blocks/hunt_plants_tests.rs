use crate::{
    block::{GetStateForNeighborUpdateArgs, OnScheduledTickArgs, RandomTickArgs},
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        EventHandler, EventPriority,
        api::events::{block::block_break::BlockBreakEvent, world::generic_game::GenericGameEvent},
    },
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use futures::future::BoxFuture;
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    biome::Biome,
    block_properties::{DriedGhastLikeProperties, HorizontalFacing},
    entity::EntityType,
    game_event::GameEvent,
    item::Item,
};
use pumpkin_protocol::{
    MultiVersionJavaPacket, VarInt,
    java::{client::play::CSoundEffect, server::play::SUseItemOn},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
};
use pumpkin_world::world::BlockFlags;
use rustc_hash::FxHashSet;
use std::sync::{Arc, Mutex};

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_registry(crate::block::registry::default_registry())
    }

    fn with_registry(registry: Arc<crate::block::registry::BlockRegistry>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(directory.path());
        Arc::get_mut(&mut server).unwrap().block_registry = registry;
        let world = combat_test_support::world(&server, directory.path());
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = TestPlayer::new(&world);
        player
            .player
            .permission_lvl
            .store(pumpkin_util::permission::PermissionLvl::Four);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 10.5));
        player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn set(&self, pos: BlockPos, state: BlockStateId) {
        self.world.set_block_state(
            &pos,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn use_block(&self, pos: BlockPos, face: BlockDirection) {
        self.player
            .client()
            .handle_use_item_on(&self.player.player, &packet(pos, face), &self.server)
            .unwrap();
    }

    fn tick(&self) {
        let active = FxHashSet::from_iter([Vector2::new(0, 0)]);
        let data = self.world.level.get_tick_data(&active, 0);
        for tick in data.block_ticks {
            let block = self.world.get_block(&tick.position);
            if block == tick.value {
                self.world
                    .block_registry
                    .get_pumpkin_block(block.id)
                    .unwrap()
                    .on_scheduled_tick(OnScheduledTickArgs {
                        world: &self.world,
                        block,
                        position: &tick.position,
                    });
            }
        }
    }

    fn random_tick(&self, pos: BlockPos) {
        let block = self.world.get_block(&pos);
        if let Some(behavior) = self.world.block_registry.get_pumpkin_block(block.id) {
            behavior.random_tick(RandomTickArgs {
                world: &self.world,
                block,
                position: &pos,
            });
        }
    }

    fn item_count(&self, item: &Item) -> u32 {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|e| e.get_item_entity())
            .map(|e| {
                let stack = e.get_item_stack().lock().unwrap();
                if stack.item == item {
                    u32::from(stack.item_count)
                } else {
                    0
                }
            })
            .sum()
    }

    fn take_sounds(&mut self) -> Vec<u16> {
        self.player
            .take_packets()
            .iter()
            .filter_map(|packet| {
                let mut data = packet.as_ref();
                (data.get_var_int().unwrap().0
                    == CSoundEffect::to_id(pumpkin_data::packet::CURRENT_MC_VERSION))
                .then(|| (data.get_var_int().unwrap().0 - 1) as u16)
            })
            .collect()
    }

    fn mine(&self, pos: BlockPos) {
        let (block, state) = self.world.get_block_and_state(&pos);
        let flags = if self.player.player.gamemode.load() == GameMode::Creative {
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS
        } else {
            BlockFlags::NOTIFY_ALL
        };
        assert!(
            self.world
                .break_block(&pos, Some(&self.player.player), flags)
                .is_some()
        );
        self.server.block_registry.broken(
            &self.world,
            block,
            &self.player.player,
            &pos,
            &self.server,
            state,
        );
    }
}

fn packet(pos: BlockPos, face: BlockDirection) -> SUseItemOn {
    SUseItemOn {
        hand: VarInt(0),
        position: pos,
        face: VarInt(face as i32),
        cursor_pos: Vector3::new(0.5, 0.5, 0.5),
        inside_block: false,
        is_against_world_border: false,
        sequence: VarInt(1),
    }
}

#[derive(Default)]
struct Events {
    breaks: Mutex<Vec<(String, bool)>>,
    changes: Mutex<Vec<String>>,
}
impl EventHandler<BlockBreakEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.breaks
                .lock()
                .unwrap()
                .push((event.block.name.to_owned(), event.drop));
        })
    }
}
impl EventHandler<GenericGameEvent> for Events {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.changes.lock().unwrap().push(event.event_key.clone());
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kelp_column_drops_items_when_support_is_removed() {
    let f = Fixture::new();
    let bottom = BlockPos::new(8, 64, 8);
    for pos in [bottom, bottom.up()] {
        f.set(pos, Block::KELP_PLANT.default_state.id);
    }
    f.set(bottom.up().up(), Block::KELP.default_state.id);
    f.world.break_block(
        &bottom.down(),
        None,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
    );
    assert_eq!(f.world.get_block(&bottom), &Block::KELP_PLANT);
    assert_eq!(f.item_count(&Item::KELP), 0);
    for step in 0..3 {
        f.tick();
        let pos = BlockPos::new(8, 64 + step, 8);
        assert_eq!(f.world.get_block(&pos), &Block::WATER);
        assert_eq!(f.item_count(&Item::KELP), (step + 1) as u32);
        if step < 2 {
            let next = if step == 0 {
                &Block::KELP_PLANT
            } else {
                &Block::KELP
            };
            assert_eq!(f.world.get_block(&pos.up()), next);
        }
    }
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn weeping_vines_cascade_through_the_normal_drop_path() {
    let f = Fixture::new();
    let top = BlockPos::new(8, 66, 8);
    f.set(top.up(), Block::STONE.default_state.id);
    f.set(top, Block::WEEPING_VINES_PLANT.default_state.id);
    f.set(top.down(), Block::WEEPING_VINES_PLANT.default_state.id);
    f.set(top.down().down(), Block::WEEPING_VINES.default_state.id);
    let events = Arc::new(Events::default());
    f.server.plugin_manager.register::<BlockBreakEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    f.world.break_block(
        &top.up(),
        None,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
    );
    assert_eq!(f.world.get_block(&top), &Block::WEEPING_VINES_PLANT);
    for step in 0..3 {
        f.tick();
        assert!(
            f.world
                .get_block_state(&BlockPos::new(8, 66 - step, 8))
                .is_air()
        );
        let breaks = events.breaks.lock().unwrap();
        // Vanilla's vine loot is random; verify each segment uses destruction with drops enabled.
        assert_eq!(
            breaks
                .iter()
                .filter(|(name, drops)| name.starts_with("weeping_vines") && *drops)
                .count(),
            (step + 1) as usize
        );
    }
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn growing_plants_keep_supported_segments_and_recheck_restored_support() {
    let f = Fixture::new();
    for (block, pos, support_direction) in [
        (&Block::KELP, BlockPos::new(4, 64, 8), BlockDirection::Down),
        (
            &Block::TWISTING_VINES,
            BlockPos::new(6, 64, 8),
            BlockDirection::Down,
        ),
        (
            &Block::WEEPING_VINES,
            BlockPos::new(8, 64, 8),
            BlockDirection::Up,
        ),
        (
            &Block::CAVE_VINES,
            BlockPos::new(10, 64, 8),
            BlockDirection::Up,
        ),
    ] {
        let support = pos.offset(support_direction.to_offset());
        f.set(support, Block::STONE.default_state.id);
        f.set(pos, block.default_state.id);
        let behavior = f.world.block_registry.get_pumpkin_block(block.id).unwrap();
        let update = || {
            behavior.get_state_for_neighbor_update(GetStateForNeighborUpdateArgs {
                world: &f.world,
                block,
                state_id: block.default_state.id,
                position: &pos,
                direction: support_direction,
                neighbor_position: &support,
                neighbor_state_id: f.world.get_block_state_id(&support),
            })
        };
        assert_eq!(update(), block.default_state.id);
        assert!(!f.world.is_block_tick_scheduled(&pos, block));
        f.set(support, BlockStateId::AIR);
        assert_eq!(update(), block.default_state.id);
        assert!(f.world.is_block_tick_scheduled(&pos, block));
        f.set(support, Block::STONE.default_state.id);
        f.tick();
        assert_eq!(f.world.get_block_state_id(&pos), block.default_state.id);
    }
    assert!(
        f.world
            .entities
            .load()
            .iter()
            .all(|e| e.get_item_entity().is_none())
    );
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cave_vines_shape_conversion_preserves_berries() {
    use pumpkin_data::block_properties::{CaveVinesLikeProperties, CaveVinesPlantLikeProperties};
    let f = Fixture::new();
    let pos = BlockPos::new(8, 66, 8);
    f.set(pos.up(), Block::STONE.default_state.id);
    let props = CaveVinesLikeProperties {
        age: 7,
        berries: true,
    };
    f.set(pos, props.to_state_id(&Block::CAVE_VINES));
    f.world.set_block_state(
        &pos.down(),
        Block::CAVE_VINES.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert_eq!(f.world.get_block(&pos), &Block::CAVE_VINES_PLANT);
    assert!(CaveVinesPlantLikeProperties::from_state_id(f.world.get_block_state_id(&pos)).berries);
    f.world.break_block(
        &pos.down(),
        None,
        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
    );
    assert_eq!(f.world.get_block(&pos), &Block::CAVE_VINES);
    let props = CaveVinesLikeProperties::from_state_id(f.world.get_block_state_id(&pos));
    assert!(props.berries);
    assert!(props.age < 25);
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waterlogged_dried_ghast_hydrates_and_spawns_a_ghastling() {
    let mut f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    let mut props = DriedGhastLikeProperties::default(&Block::DRIED_GHAST);
    props.waterlogged = true;
    props.facing = HorizontalFacing::East;
    f.set(pos, props.to_state_id(&Block::DRIED_GHAST));
    let events = Arc::new(Events::default());
    f.server.plugin_manager.register::<GenericGameEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    for hydration in 1..=3 {
        f.random_tick(pos);
        // A duplicate random tick must not queue a second hydration step.
        f.random_tick(pos);
        for _ in 0..4999 {
            f.tick();
        }
        assert_eq!(
            DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos)).hydration,
            hydration - 1
        );
        f.tick();
        assert_eq!(
            DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos)).hydration,
            hydration
        );
        assert!(!f.world.is_block_tick_scheduled(&pos, &Block::DRIED_GHAST));
    }
    assert_eq!(
        *events.changes.lock().unwrap(),
        vec![GameEvent::BlockChange.name(); 3]
    );
    f.random_tick(pos);
    for _ in 0..5000 {
        f.tick();
    }
    assert_eq!(f.world.get_block(&pos), &Block::WATER);
    let sounds = f.take_sounds();
    assert_eq!(
        sounds,
        vec![
            pumpkin_data::sound::Sound::BlockDriedGhastTransition as u16,
            pumpkin_data::sound::Sound::BlockDriedGhastTransition as u16,
            pumpkin_data::sound::Sound::BlockDriedGhastTransition as u16,
            pumpkin_data::sound::Sound::EntityGhastlingSpawn as u16,
        ]
    );
    let entities = f.world.entities.load();
    let ghastlings: Vec<_> = entities
        .iter()
        .filter(|e| e.get_entity().entity_type == &EntityType::HAPPY_GHAST)
        .collect();
    assert_eq!(ghastlings.len(), 1);
    let ghastling = ghastlings[0];
    assert!(ghastling.get_mob().unwrap().as_ageable().unwrap().is_baby());
    assert_eq!(
        ghastling.get_entity().pos.load(),
        Vector3::new(8.5, 64.0, 8.5)
    );
    assert_eq!(ghastling.get_entity().yaw.load(), -90.0);
    assert_eq!(ghastling.get_entity().head_yaw.load(), -90.0);
    assert_eq!(f.item_count(&Item::DRIED_GHAST), 0);
    assert!(f.world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dried_ghast_dry_control_and_drying_use_delayed_ticks() {
    let mut f = Fixture::new();
    let pos = BlockPos::new(8, 64, 8);
    let events = Arc::new(Events::default());
    f.server.plugin_manager.register::<GenericGameEvent, _>(
        events.clone(),
        EventPriority::Normal,
        true,
    );
    f.set(pos, Block::DRIED_GHAST.default_state.id);
    f.random_tick(pos);
    assert!(!f.world.is_block_tick_scheduled(&pos, &Block::DRIED_GHAST));
    let mut props = DriedGhastLikeProperties::default(&Block::DRIED_GHAST);
    props.hydration = 2;
    f.set(pos, props.to_state_id(&Block::DRIED_GHAST));
    for hydration in [1, 0] {
        f.random_tick(pos);
        for _ in 0..5000 {
            f.tick();
        }
        assert_eq!(
            DriedGhastLikeProperties::from_state_id(f.world.get_block_state_id(&pos)).hydration,
            hydration
        );
    }
    assert!(
        f.world
            .entities
            .load()
            .iter()
            .all(|e| e.get_entity().entity_type != &EntityType::HAPPY_GHAST)
    );
    assert_eq!(
        *events.changes.lock().unwrap(),
        vec![GameEvent::BlockChange.name(); 2]
    );
    assert!(f.take_sounds().is_empty());
    assert!(f.world.level.shutdown().await.is_ok());
}

#[path = "hunt_plants_placement_tests.rs"]
mod placement;
