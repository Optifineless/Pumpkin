#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection,
    block_properties::WhiteBedLikeProperties,
    dimension::{BedRuleOption, Dimension},
    entity::EntityPose,
    item::Item,
    item_stack::ItemStack,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::{client::play::CSystemChatMessage, server::play::SUseItemOn},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};
use pumpkin_world::{level::Level, world::BlockFlags};

use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        api::events::player::{
            player_bed::PlayerBedEnterEvent, player_spawn_change::PlayerSpawnChangeEvent,
        },
    },
    server::{Server, combat_test_support},
    world::{World, border::Worldborder},
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);
const BEDS: [(&Block, &Item); 2] = [
    (&Block::WHITE_BED, &Item::WHITE_BED),
    (&Block::STRAW_BED, &Item::STRAW_BED),
];

#[path = "bed_admission_callback_tests.rs"]
mod callbacks;

struct Callback<F>(F);
impl<E: Payload, F: Fn(&mut E) + Send + Sync> EventHandler<E> for Callback<F> {
    fn handle_blocking<'a>(&'a self, _: &'a Arc<Server>, event: &'a mut E) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    client: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new(dimension: Dimension) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = Arc::new(World::load(
            Level::from_root_folder(
                &pumpkin_config::world::LevelConfig::default(),
                directory.path().to_path_buf(),
                0,
                dimension.clone(),
            ),
            server.level_info.clone(),
            dimension,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
        world.level_time.lock().unwrap().time_of_day = 13_000;
        let client = TestPlayer::new(&world);
        client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 7.5));
        client.player.get_entity().set_rotation(-90.0, 0.0);
        Self {
            server,
            world,
            client,
            _directory: directory,
        }
    }

    fn use_on(&self, position: BlockPos) {
        self.client
            .client()
            .handle_use_item_on(
                &self.client.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
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

    fn attempt_placement(&self, foot: BlockPos, item: &'static Item) {
        for floor in [foot.down(), foot.add(1, -1, 0)] {
            self.world.set_block_state(
                &floor,
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
        self.client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, item));
        self.use_on(foot.down());
    }

    fn place(&mut self, block: &'static Block, item: &'static Item) {
        self.attempt_placement(FOOT, item);
        assert_eq!(
            self.world.get_block(&FOOT),
            block,
            "ordinary placement must succeed"
        );
        assert_eq!(self.world.get_block(&HEAD), block);
        assert!(self.client.player.inventory().held_item().is_empty());
        self.client.take_packets();
    }

    fn reset(&mut self) {
        if self.client.player.sleeping_bed_pos.load().is_some() {
            self.client.player.wake_up();
        }
        self.awake();
        for position in [HEAD.up(), FOOT.up(), HEAD, FOOT] {
            self.world.set_block_state(
                &position,
                Block::AIR.default_state.id,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }
        self.client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 7.5));
        self.client.player.get_entity().set_rotation(-90.0, 0.0);
        *self.client.player.respawn_point.lock().unwrap() = None;
        self.client.take_packets();
    }

    fn register<E: Payload + Send + Sync + 'static>(
        &self,
        callback: impl Fn(&mut E) + Send + Sync + 'static,
    ) {
        self.server.plugin_manager.register::<E, _>(
            Arc::new(Callback(callback)),
            EventPriority::Normal,
            true,
        );
    }

    fn awake(&self) {
        assert!(self.client.player.sleeping_bed_pos.load().is_none());
        assert!(self.client.player.sleeping_since.load().is_none());
        assert!(self.client.player.get_entity().pose.load() != EntityPose::Sleeping);
    }

    fn spawn(&self) -> Option<BlockPos> {
        self.client
            .player
            .respawn_point
            .lock()
            .unwrap()
            .as_ref()
            .map(|spawn| spawn.position)
    }

    fn message(&self, text: &TextComponent) -> bytes::Bytes {
        self.client
            .client()
            .serialize_packet(&CSystemChatMessage::new(text, true))
            .unwrap()
    }

    async fn finish(self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            self.world.level.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_head_border() {
    for (block, item) in BEDS {
        let fixture = Fixture::new(Dimension::OVERWORLD);
        for head_inside in [false, true] {
            *fixture.world.worldborder.lock().unwrap() =
                Worldborder::new(5.0, 5.0, if head_inside { 4.0 } else { 2.0 }, 0, 5, 15);
            fixture.attempt_placement(FOOT, item);
            assert_eq!(
                fixture.world.get_block(&FOOT) == block,
                head_inside,
                "foot inside/head outside must refuse {}",
                block.name
            );
            assert_eq!(fixture.world.get_block(&HEAD) == block, head_inside);
            assert_eq!(
                fixture.client.player.inventory().held_item().item_count,
                u8::from(!head_inside)
            );
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_reach_inclusive() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        for position in [
            Vector3::new(9.5, 64.0, 5.5),
            Vector3::new(2.5, 64.0, 5.5),
            Vector3::new(6.5, 66.0, 5.5),
            Vector3::new(6.5, 62.0, 5.5),
            Vector3::new(6.5, 64.0, 8.5),
            Vector3::new(6.5, 64.0, 2.5),
        ] {
            fixture.reset();
            fixture.place(block, item);
            fixture.client.player.get_entity().set_pos(position);
            fixture.use_on(HEAD);
            assert_eq!(
                fixture.client.player.sleeping_bed_pos.load(),
                Some(HEAD),
                "inclusive bottom-center reach at {position:?} for {}",
                block.name
            );
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_reach_outside() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        fixture.place(block, item);
        for position in [
            Vector3::new(9.5001, 64.0, 5.5),
            Vector3::new(2.4999, 64.0, 5.5),
            Vector3::new(6.5, 66.0001, 5.5),
            Vector3::new(6.5, 61.9999, 5.5),
            Vector3::new(6.5, 64.0, 8.5001),
            Vector3::new(6.5, 64.0, 2.4999),
        ] {
            fixture.client.player.get_entity().set_pos(position);
            fixture.use_on(HEAD);
            assert!(
                fixture.client.player.sleeping_bed_pos.load().is_none(),
                "outside bottom-center reach must refuse at {position:?} for {}",
                block.name
            );
            assert_eq!(
                fixture.spawn(),
                None,
                "out-of-range refusal precedes spawn mutation"
            );
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_suffocation() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        for (ceiling, suffocates) in [
            (&Block::GLASS, false),
            (&Block::STONE, true),
            (&Block::ICE, true),
            (&Block::FARMLAND, true),
            (&Block::SOUL_SAND, true),
            (&Block::DIRT_PATH, true),
            (&Block::MUD, true),
            (&Block::COPPER_GRATE, false),
            (&Block::OAK_LEAVES, false),
            (&Block::MANGROVE_ROOTS, false),
        ] {
            for covered in [FOOT, HEAD] {
                fixture.reset();
                fixture.place(block, item);
                fixture.world.set_block_state(
                    &covered.up(),
                    ceiling.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );
                fixture.use_on(HEAD);
                assert_eq!(
                    fixture.client.player.sleeping_bed_pos.load(),
                    (!suffocates).then_some(HEAD),
                    "{} suffocation={suffocates} over {covered:?} for {}",
                    ceiling.name,
                    block.name
                );
                if suffocates {
                    assert_eq!(
                        fixture.spawn(),
                        None,
                        "obstruction refusal precedes spawn mutation"
                    );
                }
            }
        }
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_dead_actor() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        fixture.place(block, item);
        fixture.client.player.living_entity.set_health(0.0);
        fixture.use_on(HEAD);
        fixture.awake();
        assert_eq!(
            fixture.spawn(),
            None,
            "dead actor cannot mutate its respawn point"
        );
        assert!(
            !WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&HEAD))
                .occupied
        );
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_already_sleeping() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(Dimension::OVERWORLD);
        fixture.place(block, item);
        let second_foot = FOOT.add(0, 0, 2);
        let second_head = HEAD.add(0, 0, 2);
        fixture
            .client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 9.5));
        fixture.attempt_placement(second_foot, item);
        assert_eq!(fixture.world.get_block(&second_head), block);
        fixture
            .client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 7.5));
        fixture.use_on(HEAD);
        assert_eq!(fixture.client.player.sleeping_bed_pos.load(), Some(HEAD));
        let timer = fixture.client.player.sleeping_since.load();
        let spawn = fixture.spawn();
        fixture.use_on(second_head);
        assert_eq!(
            fixture.client.player.sleeping_bed_pos.load(),
            Some(HEAD),
            "sleeping actor must retain its first bed"
        );
        assert_eq!(fixture.client.player.sleeping_since.load(), timer);
        assert_eq!(fixture.spawn(), spawn);
        assert!(
            !WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&second_head))
                .occupied
        );
        fixture.finish().await;
    }
}

fn denied_dimension(message: Option<&'static str>, set_spawn: bool) -> Dimension {
    let mut dimension = Dimension::OVERWORLD;
    for rule in [&mut dimension.bed_rule, &mut dimension.straw_bed_rule] {
        rule.can_sleep = BedRuleOption::Never;
        rule.can_set_spawn = if set_spawn {
            BedRuleOption::Always
        } else {
            BedRuleOption::Never
        };
        rule.error_message = message;
    }
    dimension
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_optional_message() {
    for (block, item) in BEDS {
        for message in [None, Some(r#"{"text":"custom-bed-denial"}"#)] {
            let mut fixture = Fixture::new(denied_dimension(message, false));
            fixture.place(block, item);
            fixture.use_on(HEAD);
            fixture.awake();
            assert_eq!(fixture.spawn(), None);
            let custom = fixture.message(&TextComponent::text("custom-bed-denial"));
            let packets = fixture.client.take_packets();
            let messages: Vec<_> = packets
                .iter()
                .filter(|packet| {
                    VarInt::decode(&mut packet.as_ref()).unwrap().0
                        == CSystemChatMessage::to_id(CURRENT_MC_VERSION)
                })
                .collect();
            assert_eq!(
                messages.len(),
                usize::from(message.is_some()),
                "optional denial must not invent a default message"
            );
            assert_eq!(packets.contains(&custom), message.is_some());
            fixture.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules_spawn_order() {
    for (block, item) in BEDS {
        let mut fixture = Fixture::new(denied_dimension(
            Some(r#"{"text":"too-early-to-rest"}"#),
            true,
        ));
        fixture.place(block, item);
        let spawns = Arc::new(AtomicUsize::new(0));
        let seen = spawns.clone();
        fixture.register::<PlayerSpawnChangeEvent>(move |event| {
            assert_eq!(event.new_spawn, Some(HEAD));
            assert!(event.player.sleeping_bed_pos.load().is_none());
            seen.fetch_add(1, Relaxed);
        });
        let enters = Arc::new(AtomicUsize::new(0));
        let seen = enters.clone();
        fixture.register::<PlayerBedEnterEvent>(move |_| {
            seen.fetch_add(1, Relaxed);
        });
        fixture.use_on(HEAD);
        fixture.awake();
        assert_eq!(
            fixture.spawn(),
            Some(HEAD),
            "eligible spawn mutates before sleep-time refusal for {}",
            block.name
        );
        assert_eq!(spawns.load(Relaxed), 1);
        assert_eq!(
            enters.load(Relaxed),
            0,
            "time refusal must precede bed-enter callback"
        );
        let denial = fixture.message(&TextComponent::text("too-early-to-rest"));
        assert!(fixture.client.take_packets().contains(&denial));
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_bed_admission_rules() {
    for (block, item) in BEDS {
        for too_far in [false, true] {
            let mut fixture = Fixture::new(denied_dimension(None, false));
            fixture.place(block, item);
            fixture.world.set_block_state(
                &HEAD.up(),
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
            if too_far {
                fixture
                    .client
                    .player
                    .get_entity()
                    .set_pos(Vector3::new(9.5001, 64.0, 5.5));
            }
            fixture.use_on(HEAD);
            let key = if too_far {
                "block.minecraft.bed.too_far_away"
            } else {
                "block.minecraft.bed.obstructed"
            };
            let expected = fixture.message(&TextComponent::translate(key, []));
            assert!(
                fixture.client.take_packets().contains(&expected),
                "reach then obstruction must precede optional rule denial"
            );
            fixture.awake();
            assert_eq!(fixture.spawn(), None);
            fixture.finish().await;
        }
    }
}
