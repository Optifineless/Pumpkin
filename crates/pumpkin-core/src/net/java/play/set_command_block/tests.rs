#![expect(
    clippy::unwrap_used,
    reason = "the command-block activation fixture requires successful setup"
)]

use crate::{
    block::entities::command_block::CommandBlockEntity,
    block::{BlockBehaviour, OnScheduledTickArgs, blocks::command::CommandBlock},
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
        player.player.set_gamemode(GameMode::Creative);
        player.player.permission_lvl.store(PermissionLvl::Two);
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

    fn configure(&self, mode: i32, conditional: bool) {
        let mut flags = SSetCommandBlock::FLAG_TRACK_OUTPUT | SSetCommandBlock::FLAG_AUTOMATIC;
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
    }

    fn scheduled_tick(&self) {
        let block = self.world.get_block(&COMMAND_POS);
        CommandBlock.on_scheduled_tick(OnScheduledTickArgs {
            world: &self.world,
            block,
            position: &COMMAND_POS,
        });
    }

    fn clear_scheduled_command_tick(&self) {
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&COMMAND_POS.chunk_position())
            .unwrap()
            .value()
            .clone();
        let max = BlockPos::new(
            COMMAND_POS.0.x + 1,
            COMMAND_POS.0.y + 1,
            COMMAND_POS.0.z + 1,
        );
        chunk.block_ticks.clear_area(&COMMAND_POS, &max);
    }

    fn repeating_tick_is_scheduled(&self) -> bool {
        self.world
            .level
            .loaded_chunks
            .get(&COMMAND_POS.chunk_position())
            .unwrap()
            .value()
            .block_ticks
            .is_scheduled(COMMAND_POS, &Block::REPEATING_COMMAND_BLOCK)
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
async fn always_active_impulse_evaluates_its_condition_before_first_scheduled_tick() {
    let fixture = Fixture::new(false, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(2, false);
    let configured = fixture.world.get_block_entity(&COMMAND_POS).unwrap();
    let configured = configured
        .as_any()
        .downcast_ref::<CommandBlockEntity>()
        .unwrap();
    assert!(configured.auto.load(Ordering::Acquire));
    fixture.scheduled_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conditional_impulse_does_not_run_when_its_predecessor_failed() {
    let fixture = Fixture::new(true, false, 0);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(2, true);
    let configured = fixture.world.get_block_entity(&COMMAND_POS).unwrap();
    let configured = configured
        .as_any()
        .downcast_ref::<CommandBlockEntity>()
        .unwrap();
    assert!(!configured.condition_met.load(Ordering::Acquire));
    fixture.scheduled_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(!configured.condition_met.load(Ordering::Acquire));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeating_block_uses_the_previous_condition_and_reschedules_after_each_tick() {
    let fixture = Fixture::new(true, true, 1);
    let executions = Arc::new(AtomicUsize::new(0));
    fixture.register_probe(executions.clone());

    fixture.configure(1, true);
    let block = fixture.world.get_block(&COMMAND_POS);
    assert_eq!(block.id, Block::REPEATING_COMMAND_BLOCK.id);
    // The first run consumes the condition captured at activation; this change only
    // affects the condition for the following run.
    fixture
        .predecessor
        .success_count
        .store(0, Ordering::Release);

    // Consume the activation tick from the fixture queue, then invoke the real callback.
    fixture.clear_scheduled_command_tick();
    fixture.scheduled_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    let configured = fixture.world.get_block_entity(&COMMAND_POS).unwrap();
    let configured = configured
        .as_any()
        .downcast_ref::<CommandBlockEntity>()
        .unwrap();
    assert!(!configured.condition_met.load(Ordering::Acquire));
    assert!(configured.auto.load(Ordering::Acquire));
    assert!(fixture.repeating_tick_is_scheduled());

    // Consume the requeued tick to model the next scheduled invocation.
    fixture.clear_scheduled_command_tick();
    fixture.scheduled_tick();
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert!(!configured.condition_met.load(Ordering::Acquire));
    assert!(configured.auto.load(Ordering::Acquire));
    assert!(fixture.repeating_tick_is_scheduled());
    fixture.finish().await;
}
