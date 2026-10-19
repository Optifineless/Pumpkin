#![expect(
    clippy::unwrap_used,
    reason = "the command-block activation fixture requires successful setup"
)]

use crate::{
    block::entities::command_block::CommandBlockEntity,
    command::{
        argument_builder::{ArgumentBuilder, command},
        context::command_context::CommandContext,
        node::{CommandExecutor, CommandExecutorResult},
    },
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockStateId,
    biome::Biome,
    block_properties::{CommandBlockLikeProperties, Facing},
};
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SSetCommandBlock};
use pumpkin_util::{GameMode, PermissionLvl, math::position::BlockPos, math::vector2::Vector2};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const COMMAND_POS: BlockPos = BlockPos::new(8, 64, 8);
const PREDECESSOR_POS: BlockPos = BlockPos::new(8, 64, 9);
const PROBE_COMMAND: &str = "issue159_probe";

struct Probe(Arc<AtomicUsize>);

impl CommandExecutor for Probe {
    fn execute(&self, _context: &CommandContext) -> CommandExecutorResult {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(1)
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    predecessor: Arc<CommandBlockEntity>,
}

impl Fixture {
    fn new(conditional: bool, initial_condition_met: bool, predecessor_success: u32) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        server.level_info.rcu(|level_info| {
            let mut level_info = (**level_info).clone();
            level_info.game_rules.random_tick_speed = 0;
            level_info.game_rules.spawn_mobs = false;
            level_info
        });
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        chunk.set_block_state(
            COMMAND_POS.0.x,
            COMMAND_POS.0.y,
            COMMAND_POS.0.z,
            command_block_state(&Block::COMMAND_BLOCK, conditional).to_state(),
        );
        chunk.set_block_state(
            PREDECESSOR_POS.0.x,
            PREDECESSOR_POS.0.y,
            PREDECESSOR_POS.0.z,
            command_block_state(&Block::COMMAND_BLOCK, false).to_state(),
        );
        spawn_test_support::publish(&world, chunk);
        world
            .active_chunks
            .write()
            .unwrap()
            .insert(Vector2::new(0, 0));

        let command_block = Arc::new(CommandBlockEntity::new(COMMAND_POS, true, false));
        command_block
            .condition_met
            .store(initial_condition_met, Ordering::Relaxed);
        let predecessor = Arc::new(CommandBlockEntity::new(PREDECESSOR_POS, true, false));
        predecessor
            .success_count
            .store(predecessor_success, Ordering::Relaxed);
        world.add_block_entity(command_block);
        world.add_block_entity(predecessor.clone());

        let player = TestPlayer::new(&world);
        // Player::set_gamemode consults the server plugin event hook; these packet
        // fixtures need only expose the same active Creative state to the handler.
        player.player.gamemode.store(GameMode::Creative);
        player.player.permission_lvl.store(PermissionLvl::Two);
        assert!(player.player.is_creative());
        assert!(player.player.permission_lvl.load() >= PermissionLvl::Two);
        Self {
            _directory: directory,
            server,
            world,
            player,
            predecessor,
        }
    }

    fn register_probe(&self, executions: Arc<AtomicUsize>) {
        let mut dispatcher = (*self.server.command_dispatcher.load_full()).clone();
        dispatcher.register(
            command(PROBE_COMMAND, "command-block activation regression probe")
                .executes(Probe(executions)),
        );
        self.server.command_dispatcher.store(Arc::new(dispatcher));
    }

    fn configure(&self, mode: i32, conditional: bool, automatic: bool) {
        let mut flags = SSetCommandBlock::FLAG_TRACK_OUTPUT;
        if automatic {
            flags |= SSetCommandBlock::FLAG_AUTOMATIC;
        }
        if conditional {
            flags |= SSetCommandBlock::FLAG_CONDITIONAL;
        }
        let packet = SSetCommandBlock {
            pos: COMMAND_POS,
            command: PROBE_COMMAND,
            mode: VarInt(mode),
            flags,
        };
        self.player
            .client()
            .handle_set_command_block(&self.player.player, &packet);
        let block_entity = self.world.get_block_entity(&COMMAND_POS).unwrap();
        let command_block = block_entity
            .as_any()
            .downcast_ref::<CommandBlockEntity>()
            .unwrap();
        assert_eq!(*command_block.command.lock().unwrap(), PROBE_COMMAND);
    }

    fn set_powered(&self, powered: bool) {
        let block_entity = self.world.get_block_entity(&COMMAND_POS).unwrap();
        let command_block = block_entity
            .as_any()
            .downcast_ref::<CommandBlockEntity>()
            .unwrap();
        command_block.powered.store(powered, Ordering::Release);
    }

    fn condition_met(&self) -> bool {
        let block_entity = self.world.get_block_entity(&COMMAND_POS).unwrap();
        block_entity
            .as_any()
            .downcast_ref::<CommandBlockEntity>()
            .unwrap()
            .condition_met
            .load(Ordering::Acquire)
    }

    fn persisted_condition_met(&self) -> bool {
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&COMMAND_POS.chunk_position())
            .unwrap()
            .value()
            .clone();
        chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&COMMAND_POS)
            .unwrap()
            .get_bool("conditionMet")
            .unwrap()
    }

    fn tick_is_scheduled(&self, block: &Block) -> bool {
        self.world.is_block_tick_scheduled(&COMMAND_POS, block)
    }

    fn advance_tick(&self) {
        self.server.tick();
        self.server.update_tick_times(0);
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn command_block_state(block: &Block, conditional: bool) -> BlockStateId {
    let properties = CommandBlockLikeProperties {
        conditional,
        facing: Facing::North,
    };
    properties.to_state_id(block)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn always_active_impulse_captures_conditional_predicate_before_scheduled_tick() {
    for (predecessor_success, expected_executions) in [(1u32, 1usize), (0, 0)] {
        let fixture = Fixture::new(true, false, predecessor_success);
        let executions = Arc::new(AtomicUsize::new(0));
        fixture.register_probe(executions.clone());

        fixture.configure(2, true, true);
        assert_eq!(fixture.condition_met(), predecessor_success > 0);
        assert_eq!(fixture.persisted_condition_met(), predecessor_success > 0);
        assert!(fixture.tick_is_scheduled(&Block::COMMAND_BLOCK));
        fixture.advance_tick();
        assert_eq!(executions.load(Ordering::Relaxed), expected_executions);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn always_active_repeating_consumes_activation_snapshot_then_refreshes_next_tick() {
    for (predecessor_success, expected_first_execution) in [(1u32, 1usize), (0, 0)] {
        let fixture = Fixture::new(true, false, predecessor_success);
        let executions = Arc::new(AtomicUsize::new(0));
        fixture.register_probe(executions.clone());

        fixture.configure(1, true, true);
        assert_eq!(fixture.condition_met(), predecessor_success > 0);
        assert_eq!(fixture.persisted_condition_met(), predecessor_success > 0);
        assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
        if predecessor_success > 0 {
            fixture
                .predecessor
                .success_count
                .store(0, Ordering::Release);
            assert!(fixture.condition_met());
        }

        fixture.advance_tick();
        assert_eq!(executions.load(Ordering::Relaxed), expected_first_execution);
        assert!(!fixture.condition_met());
        assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
        fixture.advance_tick();
        assert_eq!(executions.load(Ordering::Relaxed), expected_first_execution);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_automatic_save_preserves_queued_false_predicate() {
    let fixture = Fixture::new(true, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(1, true, true);
    assert!(!fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(!fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));

    fixture
        .predecessor
        .success_count
        .store(1, Ordering::Release);
    fixture.configure(1, true, true);
    assert!(!fixture.condition_met());

    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_automatic_save_preserves_queued_true_predicate() {
    let fixture = Fixture::new(true, false, 1);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(1, true, true);
    assert!(fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert!(fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));

    fixture
        .predecessor
        .success_count
        .store(0, Ordering::Release);
    fixture.configure(1, true, true);
    assert!(fixture.condition_met());

    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 2);
    assert!(!fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_repeating_to_impulse_does_not_schedule_another_execution() {
    let fixture = Fixture::new(false, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(1, false, true);
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    let queued_predicate = fixture.condition_met();

    fixture.configure(2, false, true);
    assert_eq!(fixture.condition_met(), queued_predicate);
    assert!(!fixture.tick_is_scheduled(&Block::COMMAND_BLOCK));
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entering_repeating_while_automatic_schedules_activation() {
    let fixture = Fixture::new(false, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(2, false, true);
    fixture.configure(1, false, true);
    assert!(fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entering_repeating_while_powered_schedules_activation() {
    let fixture = Fixture::new(false, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());
    fixture.set_powered(true);

    fixture.configure(1, false, false);
    assert!(fixture.condition_met());
    assert!(fixture.tick_is_scheduled(&Block::REPEATING_COMMAND_BLOCK));
    fixture.advance_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    fixture.finish().await;
}
