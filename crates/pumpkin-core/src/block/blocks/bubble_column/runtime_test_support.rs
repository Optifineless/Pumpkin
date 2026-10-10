use crate::{
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::block::block_physics::BlockPhysicsEvent,
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockStateId,
    biome::Biome,
    block_properties::{BubbleColumnLikeProperties, WhiteWoolSlabLikeProperties},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{chunk::ChunkData, world::BlockFlags};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

pub const SUPPORT: BlockPos = BlockPos::new(8, 63, 8);

pub fn cell(index: usize) -> BlockPos {
    BlockPos::new(8, 64 + i32::try_from(index).unwrap(), 8)
}

pub fn water() -> BlockStateId {
    Block::WATER.default_state.id
}

pub fn bubble(drag_down: bool) -> BlockStateId {
    BubbleColumnLikeProperties { drag: drag_down }.to_state_id(&Block::BUBBLE_COLUMN)
}

pub fn waterlogged_slab() -> BlockStateId {
    let mut properties = WhiteWoolSlabLikeProperties::default(&Block::OAK_SLAB);
    properties.waterlogged = true;
    properties.to_state_id(&Block::OAK_SLAB)
}

pub struct RuntimeFixture {
    pub server: Arc<Server>,
    pub world: Arc<World>,
    chunk: Arc<ChunkData>,
    height: usize,
    _directory: tempfile::TempDir,
}

impl RuntimeFixture {
    pub fn new(support: &Block, states: &[BlockStateId]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.random_tick_speed = 0;
            info.game_rules.spawn_mobs = false;
            info
        });
        let world = combat_test_support::world(&server, directory.path());
        let mut proto = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        proto.set_block_state(8, 63, 8, support.default_state);
        for (index, state) in states.iter().enumerate() {
            let pos = cell(index);
            proto.set_block_state(pos.0.x, pos.0.y, pos.0.z, state.to_state());
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                proto.set_block_state(8 + dx, pos.0.y, 8 + dz, Block::STONE.default_state);
            }
        }
        let cap = cell(states.len());
        proto.set_block_state(cap.0.x, cap.0.y, cap.0.z, Block::STONE.default_state);
        let chunk = spawn_test_support::publish(&world, proto);
        world
            .active_chunks
            .write()
            .unwrap()
            .insert(Vector2::new(0, 0));
        let fixture = Self {
            server,
            world,
            chunk,
            height: states.len(),
            _directory: directory,
        };
        assert!(
            fixture.queues_empty(),
            "proto seeding must not run callbacks"
        );
        fixture
    }

    pub fn settled_water(height: usize) -> Self {
        let fixture = Self::new(&Block::STONE, &vec![BlockStateId::AIR; height]);
        for index in 0..height {
            fixture.put(index, water(), BlockFlags::NOTIFY_ALL);
        }
        // Consume the old placement ticks too, so they cannot mask a missing support trigger.
        fixture.advance(20);
        assert_eq!(fixture.states(), vec![water(); height]);
        assert!(
            fixture.queues_empty(),
            "closed water shaft must settle before the action"
        );
        fixture
    }

    pub fn put(&self, index: usize, state: BlockStateId, flags: BlockFlags) {
        self.world.set_block_state(&cell(index), state, flags);
    }

    pub fn replace_support(&self, block: &Block, flags: BlockFlags) {
        self.world
            .set_block_state(&SUPPORT, block.default_state.id, flags);
    }

    pub fn advance(&self, count: u8) {
        for _ in 0..count {
            self.world.tick_chunks(&self.server);
        }
    }

    pub fn states(&self) -> Vec<BlockStateId> {
        (0..self.height)
            .map(|index| self.world.get_block_state_id(&cell(index)))
            .collect()
    }

    pub fn queues_empty(&self) -> bool {
        !self.chunk.block_ticks.has_ticks() && !self.chunk.fluid_ticks.has_ticks()
    }

    pub fn watch_physics(&self) -> Arc<PhysicsMonitor> {
        let monitor = Arc::new(PhysicsMonitor {
            column: (0..self.height).map(cell).collect(),
            support_calls: AtomicUsize::new(0),
            propagation_calls: AtomicUsize::new(0),
            cancel_support: AtomicBool::new(false),
        });
        self.server.plugin_manager.register::<BlockPhysicsEvent, _>(
            monitor.clone(),
            EventPriority::Normal,
            true,
        );
        monitor
    }

    pub async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

pub struct PhysicsMonitor {
    column: Vec<BlockPos>,
    pub support_calls: AtomicUsize,
    pub propagation_calls: AtomicUsize,
    pub cancel_support: AtomicBool,
}

impl EventHandler<BlockPhysicsEvent> for PhysicsMonitor {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block_pos == cell(0) && event.changed_pos == SUPPORT {
            self.support_calls.fetch_add(1, Relaxed);
            event.cancelled = self.cancel_support.load(Relaxed);
        }
        // Only changes originating in this shaft count, excluding setup and support writes.
        if self.column.contains(&event.changed_pos) {
            self.propagation_calls.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}
