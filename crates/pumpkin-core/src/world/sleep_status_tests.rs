#![expect(
    clippy::unwrap_used,
    reason = "Night-skip regressions use valid in-memory server fixtures"
)]

use std::sync::Arc;

use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{BedPart, HorizontalFacing, WhiteBedLikeProperties},
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
};

use crate::{
    entity::{EntityBase, player::Player},
    net::java::combat_test_support::TestPlayer,
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};

const FIRST_BED: BlockPos = BlockPos::new(5, 64, 5);
const SECOND_BED: BlockPos = BlockPos::new(9, 64, 5);
const NIGHT: i64 = 13_000;
const NEXT_DAY_AFTER_TICK: i64 = 24_001;
const SLEEP_TICKS: u8 = 100;

struct NightSkipFixture {
    server: Arc<Server>,
    world: Arc<World>,
    registered_players: Vec<Arc<Player>>,
    _directory: tempfile::TempDir,
}

impl NightSkipFixture {
    fn new(sleeping_percentage: Option<i64>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(directory.path());
        Arc::get_mut(&mut server)
            .unwrap()
            .basic_config
            .spawn_protection = 0;
        server.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.advance_weather = false;
            if let Some(percentage) = sleeping_percentage {
                info.game_rules.players_sleeping_percentage = percentage;
            }
            info
        });
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));

        let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
        add_ordinary_bed(&mut chunk, FIRST_BED);
        add_ordinary_bed(&mut chunk, SECOND_BED);
        spawn_test_support::publish(&world, chunk);
        world
            .weather
            .lock()
            .unwrap()
            .set_weather_parameters(&world, 0, 10_000, false, false);
        world.set_time_of_day(NIGHT);

        Self {
            server,
            world,
            registered_players: Vec::new(),
            _directory: directory,
        }
    }

    fn add_player(&mut self, position: Vector3<f64>) -> TestPlayer {
        let player = TestPlayer::new(&self.world);
        player.player.get_entity().set_pos(position);
        self.registered_players.push(player.player.clone());
        self.world
            .players
            .store(Arc::new(self.registered_players.clone()));
        player
    }

    fn add_player_at_bed(&mut self, foot: BlockPos) -> TestPlayer {
        self.add_player(position_near_bed(foot))
    }

    fn admit_sleep(&self, player: &TestPlayer, foot: BlockPos, sequence: i32) {
        player
            .client()
            .handle_use_item_on(
                &player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position: foot,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(sequence),
                },
                &self.server,
            )
            .unwrap();
        assert_eq!(player.player.sleeping_bed_pos.load(), Some(bed_head(foot)));
        assert_eq!(player.player.sleeping_since.load(), Some(0));
    }

    fn tick_player(&self, player: &TestPlayer, ticks: u8) {
        for _ in 0..ticks {
            player.player.tick(&self.server);
        }
    }

    async fn finish(self) {
        self.world.level.shutdown().await.unwrap();
    }
}

fn add_ordinary_bed(
    chunk: &mut pumpkin_world::generation::proto_chunk::ProtoChunk,
    foot: BlockPos,
) {
    let facing = HorizontalFacing::East;
    let mut foot_properties = WhiteBedLikeProperties::default(&Block::WHITE_BED);
    foot_properties.part = BedPart::Foot;
    foot_properties.facing = facing;
    let foot_state = foot_properties.to_state_id(&Block::WHITE_BED);
    chunk.set_block_state(foot.0.x, foot.0.y, foot.0.z, foot_state.to_state());

    let head = bed_head(foot);
    let mut head_properties = WhiteBedLikeProperties::default(&Block::WHITE_BED);
    head_properties.part = BedPart::Head;
    head_properties.facing = facing;
    let head_state = head_properties.to_state_id(&Block::WHITE_BED);
    chunk.set_block_state(head.0.x, head.0.y, head.0.z, head_state.to_state());
}

fn bed_head(foot: BlockPos) -> BlockPos {
    foot.offset(HorizontalFacing::East.to_offset())
}

fn position_near_bed(foot: BlockPos) -> Vector3<f64> {
    Vector3::new(
        f64::from(foot.0.x) + 0.5,
        f64::from(foot.0.y),
        f64::from(foot.0.z) + 1.5,
    )
}

fn set_gamemode(player: &TestPlayer, gamemode: GameMode) {
    assert!(player.player.set_gamemode(gamemode));
    assert_eq!(player.player.gamemode.load(), gamemode);
}

fn assert_stayed_at_night(world: &World) {
    assert_eq!(world.get_time_of_day(), NIGHT + 1);
}

fn assert_advanced_to_next_day(world: &World) {
    assert_eq!(world.get_time_of_day(), NEXT_DAY_AFTER_TICK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g01_spectator_does_not_block_mature_sleeper() {
    let mut fixture = NightSkipFixture::new(None);
    let sleeper = fixture.add_player_at_bed(FIRST_BED);
    fixture.admit_sleep(&sleeper, FIRST_BED, 1);

    fixture.tick_player(&sleeper, SLEEP_TICKS - 1);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(99));
    fixture.world.tick_environment();
    assert_stayed_at_night(&fixture.world);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(99));

    fixture.tick_player(&sleeper, 1);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));
    fixture.world.tick_environment();
    assert_advanced_to_next_day(&fixture.world);
    assert_eq!(sleeper.player.sleeping_since.load(), None);
    assert_eq!(sleeper.player.sleeping_bed_pos.load(), None);

    fixture.world.set_time_of_day(NIGHT);
    sleeper
        .player
        .get_entity()
        .set_pos(position_near_bed(FIRST_BED));
    let spectator = fixture.add_player(Vector3::new(12.5, 64.0, 8.5));
    set_gamemode(&spectator, GameMode::Spectator);
    fixture.admit_sleep(&sleeper, FIRST_BED, 2);
    fixture.tick_player(&sleeper, SLEEP_TICKS);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));

    fixture.world.tick_environment();
    assert_advanced_to_next_day(&fixture.world);
    assert_eq!(sleeper.player.sleeping_since.load(), None);
    assert_eq!(sleeper.player.sleeping_bed_pos.load(), None);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g01_spectator_switch_wakes_mature_sleeper() {
    let mut fixture = NightSkipFixture::new(None);
    let sleeper = fixture.add_player_at_bed(FIRST_BED);
    fixture.admit_sleep(&sleeper, FIRST_BED, 1);
    fixture.tick_player(&sleeper, SLEEP_TICKS);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));

    set_gamemode(&sleeper, GameMode::Spectator);
    assert_eq!(sleeper.player.sleeping_since.load(), None);
    assert_eq!(sleeper.player.sleeping_bed_pos.load(), None);
    assert!(sleeper.player.get_entity().pose.load() != pumpkin_data::entity::EntityPose::Sleeping);

    set_gamemode(&sleeper, GameMode::Survival);
    fixture.world.tick_environment();
    assert_stayed_at_night(&fixture.world);

    fixture.admit_sleep(&sleeper, FIRST_BED, 2);
    fixture.tick_player(&sleeper, SLEEP_TICKS);
    fixture.world.tick_environment();
    assert_advanced_to_next_day(&fixture.world);
    fixture.finish().await;
}

async fn percentage_case(sleeping_gamemode: GameMode) -> bool {
    let mut fixture = NightSkipFixture::new(Some(50));
    let sleeper = fixture.add_player_at_bed(FIRST_BED);
    let awake = fixture.add_player_at_bed(SECOND_BED);
    let spectator = fixture.add_player(Vector3::new(12.5, 64.0, 8.5));
    if sleeping_gamemode == GameMode::Creative {
        set_gamemode(&sleeper, GameMode::Creative);
    } else {
        set_gamemode(&awake, GameMode::Creative);
    }
    set_gamemode(&spectator, GameMode::Spectator);

    fixture.admit_sleep(&sleeper, FIRST_BED, 1);
    fixture.tick_player(&sleeper, SLEEP_TICKS);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));
    fixture.world.tick_environment();

    let advanced = fixture.world.get_time_of_day() == NEXT_DAY_AFTER_TICK;
    let woke = sleeper.player.sleeping_since.load().is_none();
    fixture.finish().await;
    advanced && woke
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g01_percentage_uses_active_population() {
    let survival_sleeper = percentage_case(GameMode::Survival).await;
    let creative_sleeper = percentage_case(GameMode::Creative).await;
    assert!(
        survival_sleeper,
        "one mature Survival sleeper must meet 50% of two active players"
    );
    assert!(
        creative_sleeper,
        "one mature Creative sleeper must meet 50% of two active players"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g01_gamemode_change_recounts_requirement() {
    let mut fixture = NightSkipFixture::new(None);
    let sleeper = fixture.add_player_at_bed(FIRST_BED);
    let awake = fixture.add_player_at_bed(SECOND_BED);
    fixture.admit_sleep(&sleeper, FIRST_BED, 1);
    fixture.tick_player(&sleeper, SLEEP_TICKS);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));

    fixture.world.tick_environment();
    assert_stayed_at_night(&fixture.world);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));

    set_gamemode(&awake, GameMode::Spectator);
    fixture.world.tick_environment();
    assert_advanced_to_next_day(&fixture.world);
    assert_eq!(sleeper.player.sleeping_since.load(), None);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g01_negative_controls() {
    let empty = NightSkipFixture::new(None);
    empty.world.tick_environment();
    assert_stayed_at_night(&empty.world);
    empty.finish().await;

    let mut spectators_only = NightSkipFixture::new(None);
    let spectator = spectators_only.add_player(Vector3::new(12.5, 64.0, 8.5));
    set_gamemode(&spectator, GameMode::Spectator);
    spectators_only.world.tick_environment();
    assert_stayed_at_night(&spectators_only.world);
    spectators_only.finish().await;

    let mut zero_percent = NightSkipFixture::new(Some(0));
    let awake = zero_percent.add_player_at_bed(FIRST_BED);
    zero_percent.world.tick_environment();
    assert_stayed_at_night(&zero_percent.world);
    assert_eq!(awake.player.sleeping_since.load(), None);
    zero_percent.finish().await;

    let mut ninety_nine_ticks = NightSkipFixture::new(None);
    let sleeper = ninety_nine_ticks.add_player_at_bed(FIRST_BED);
    ninety_nine_ticks.admit_sleep(&sleeper, FIRST_BED, 1);
    ninety_nine_ticks.tick_player(&sleeper, SLEEP_TICKS - 1);
    ninety_nine_ticks.world.tick_environment();
    assert_stayed_at_night(&ninety_nine_ticks.world);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(99));
    ninety_nine_ticks.finish().await;

    let mut awake_blocker = NightSkipFixture::new(None);
    let sleeper = awake_blocker.add_player_at_bed(FIRST_BED);
    let _awake = awake_blocker.add_player_at_bed(SECOND_BED);
    awake_blocker.admit_sleep(&sleeper, FIRST_BED, 1);
    awake_blocker.tick_player(&sleeper, SLEEP_TICKS);
    awake_blocker.world.tick_environment();
    assert_stayed_at_night(&awake_blocker.world);
    assert_eq!(sleeper.player.sleeping_since.load(), Some(100));
    awake_blocker.finish().await;

    let mut enough_sleepers = NightSkipFixture::new(None);
    let first = enough_sleepers.add_player_at_bed(FIRST_BED);
    let second = enough_sleepers.add_player_at_bed(SECOND_BED);
    enough_sleepers.admit_sleep(&first, FIRST_BED, 1);
    enough_sleepers.admit_sleep(&second, SECOND_BED, 1);
    enough_sleepers.tick_player(&first, SLEEP_TICKS);
    enough_sleepers.tick_player(&second, SLEEP_TICKS);
    enough_sleepers.world.tick_environment();
    assert_advanced_to_next_day(&enough_sleepers.world);
    assert_eq!(first.player.sleeping_since.load(), None);
    assert_eq!(second.player.sleeping_since.load(), None);
    enough_sleepers.finish().await;
}
