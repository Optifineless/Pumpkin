#![expect(clippy::unwrap_used, reason = "Regression fixtures must be valid")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockDirection, BlockStateId, dimension::Dimension, entity::EntityPose, item::Item,
    item_stack::ItemStack, packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::{
        client::play::{CExplosion, CSystemChatMessage},
        server::play::SUseItemOn,
    },
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};
use pumpkin_world::{level::Level, world::BlockFlags};

use super::World;
use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, Payload,
        api::events::{
            block::block_break::BlockBreakEvent, entity::entity_explode::EntityExplodeEvent,
            player::player_interact_event::PlayerInteractEvent,
        },
    },
    server::{Server, combat_test_support},
};

const FOOT: BlockPos = BlockPos::new(5, 64, 5);
const HEAD: BlockPos = BlockPos::new(6, 64, 5);

#[path = "bed_use_round3_tests.rs"]
mod round3;

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
        Self::with_registry(dimension, None)
    }

    fn with_registry(
        dimension: Dimension,
        registry: Option<Arc<crate::block::registry::BlockRegistry>>,
    ) -> Self {
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
            registry.unwrap_or_else(|| server.block_registry.clone()),
            Arc::downgrade(&server),
        ));
        combat_test_support::publish_empty_chunk(&world, Vector2::new(0, 0));
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

    fn place(&mut self, block: &'static Block, item: &'static Item) {
        for position in [FOOT.down(), HEAD.down()] {
            self.world.set_block_state(
                &position,
                Block::STONE.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
        self.client
            .player
            .inventory()
            .set_stack(0, ItemStack::new(1, item));
        self.use_on(FOOT.down());
        assert_eq!(self.world.get_block(&FOOT), block);
        assert_eq!(self.world.get_block(&HEAD), block);
        assert!(self.client.player.inventory().held_item().is_empty());
        // Keep the real placement, but leave no floor on which randomized explosion fire can form.
        for position in [FOOT.down(), HEAD.down()] {
            self.world.set_block_state(
                &position,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }
        self.client.take_packets();
    }

    fn awake(&self) {
        assert!(self.client.player.sleeping_bed_pos.load().is_none());
        assert!(self.client.player.sleeping_since.load().is_none());
        assert!(self.client.player.get_entity().pose.load() != EntityPose::Sleeping);
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

fn explosion_packets(packets: &[bytes::Bytes]) -> usize {
    packets
        .iter()
        .filter(|packet| {
            VarInt::decode(&mut packet.as_ref()).unwrap().0 == CExplosion::to_id(CURRENT_MC_VERSION)
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_nether_beds_destroy_before_sleep_admission() {
    for time in [1_000, 13_000] {
        for clicked in [FOOT, HEAD] {
            for mode in [GameMode::Survival, GameMode::Creative] {
                let mut fixture = Fixture::new(Dimension::THE_NETHER);
                fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
                fixture.world.level_time.lock().unwrap().time_of_day = time;
                fixture.client.player.gamemode.store(mode);
                // Keep explosion damage from turning a bed-use regression into a death fixture.
                fixture.client.player.set_invulnerable(true);
                let explosions = Arc::new(AtomicUsize::new(0));
                let seen = explosions.clone();
                let world = fixture.world.clone();
                fixture.register::<EntityExplodeEvent>(move |event| {
                    assert_eq!(event.position, HEAD.to_centered_f64());
                    assert_eq!(event.yield_rate, 5.0);
                    assert!(world.get_block(&HEAD).is_air());
                    assert!(world.get_block(&FOOT).is_air());
                    seen.fetch_add(1, Relaxed);
                });
                let no_sleep = TextComponent::translate("block.minecraft.bed.no_sleep", []);
                let no_sleep_packet = fixture
                    .client
                    .client()
                    .serialize_packet(&CSystemChatMessage::new(&no_sleep, true))
                    .unwrap();
                fixture.use_on(clicked);
                assert_eq!(
                    explosions.load(Relaxed),
                    1,
                    "unsafe bed must explode at time={time}, clicked={clicked:?}"
                );
                assert!(fixture.world.get_block(&HEAD).is_air());
                assert!(fixture.world.get_block(&FOOT).is_air());
                fixture.awake();
                let packets = fixture.client.take_packets();
                assert_eq!(explosion_packets(&packets), 1);
                assert!(!packets.contains(&no_sleep_packet));
                fixture.finish().await;
            }
        }
    }
    let mut custom_rule = Dimension::THE_NETHER;
    custom_rule.bed_rule.error_message = Some(r#"{"text":"unsafe-rule-message"}"#);
    let mut fixture = Fixture::new(custom_rule);
    fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
    fixture.client.player.set_invulnerable(true);
    fixture.use_on(HEAD);
    let message = TextComponent::text("unsafe-rule-message");
    let overlay = fixture
        .client
        .client()
        .serialize_packet(&CSystemChatMessage::new(&message, true))
        .unwrap();
    let packets = fixture.client.take_packets();
    assert!(packets.contains(&overlay));
    assert_eq!(explosion_packets(&packets), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_bed_species_keep_distinct_rules() {
    for (dimension, block, item, sleeps) in [
        (
            Dimension::OVERWORLD,
            &Block::WHITE_BED,
            &Item::WHITE_BED,
            true,
        ),
        (
            Dimension::OVERWORLD,
            &Block::STRAW_BED,
            &Item::STRAW_BED,
            true,
        ),
        (
            Dimension::THE_NETHER,
            &Block::STRAW_BED,
            &Item::STRAW_BED,
            false,
        ),
    ] {
        let mut fixture = Fixture::new(dimension);
        fixture.place(block, item);
        fixture.world.level_time.lock().unwrap().time_of_day = 13_000;
        fixture.use_on(FOOT);
        assert_eq!(
            fixture.client.player.sleeping_bed_pos.load(),
            sleeps.then_some(HEAD)
        );
        assert_eq!(fixture.world.get_block(&HEAD).is_air(), !sleeps);
        assert_eq!(fixture.world.get_block(&FOOT).is_air(), !sleeps);
        assert_eq!(explosion_packets(&fixture.client.take_packets()), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_unsafe_use_cancellation_preserves_both_halves() {
    for cancel in 0..3 {
        let mut fixture = Fixture::new(Dimension::THE_NETHER);
        fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
        let states = [
            fixture.world.get_block_state_id(&HEAD),
            fixture.world.get_block_state_id(&FOOT),
        ];
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let world = fixture.world.clone();
        fixture.register::<BlockBreakEvent>(move |event| {
            assert!(event.player.is_none());
            assert_eq!(event.block, &Block::WHITE_BED);
            assert_eq!(event.exp, 0);
            assert!(!event.drop);
            let index = seen.fetch_add(1, Relaxed);
            assert_eq!(event.block_position, [HEAD, FOOT][index]);
            assert_eq!(world.get_block_state_id(&HEAD), states[0]);
            assert_eq!(world.get_block_state_id(&FOOT), states[1]);
            event.cancelled = (cancel == 1 && event.block_position == HEAD)
                || (cancel == 2 && event.block_position == FOOT);
        });
        if cancel == 0 {
            fixture.register::<PlayerInteractEvent>(|event| event.cancelled = true);
        }
        fixture.use_on(FOOT);
        assert_eq!(calls.load(Relaxed), cancel);
        assert_eq!(fixture.world.get_block_state_id(&HEAD), states[0]);
        assert_eq!(fixture.world.get_block_state_id(&FOOT), states[1]);
        fixture.awake();
        assert_eq!(explosion_packets(&fixture.client.take_packets()), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_unsafe_bed_use_reentry_admits_one_pair_operation() {
    let mut fixture = Fixture::new(Dimension::THE_NETHER);
    fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
    fixture.client.player.set_invulnerable(true);

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let reentrant_player = fixture.client.player.clone();
    let reentrant_client = reentrant_player.client.clone();
    let server = fixture.server.clone();
    fixture.register::<BlockBreakEvent>(move |_| {
        if seen.fetch_add(1, Relaxed) == 0
            && let crate::net::ClientPlatform::Java(client) = reentrant_client.as_ref()
        {
            client
                .handle_use_item_on(
                    &reentrant_player,
                    &SUseItemOn {
                        hand: VarInt(0),
                        position: FOOT,
                        face: VarInt(BlockDirection::Up as i32),
                        cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                        inside_block: false,
                        is_against_world_border: false,
                        sequence: VarInt(2),
                    },
                    &server,
                )
                .unwrap();
        }
    });

    fixture.use_on(FOOT);

    assert_eq!(calls.load(Relaxed), 2, "one operation admits both halves");
    assert!(fixture.world.get_block(&HEAD).is_air());
    assert!(fixture.world.get_block(&FOOT).is_air());
    assert_eq!(explosion_packets(&fixture.client.take_packets()), 1);
    fixture.awake();
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_unsafe_break_callbacks_preserve_replacement_containers() {
    for replacement in [HEAD, FOOT] {
        let mut fixture = Fixture::new(Dimension::THE_NETHER);
        fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
        let partner = if replacement == HEAD { FOOT } else { HEAD };
        let partner_state = fixture.world.get_block_state_id(&partner);
        let world = fixture.world.clone();
        fixture.register::<BlockBreakEvent>(move |event| {
            if event.block_position == HEAD {
                world.set_block_state(
                    &replacement,
                    Block::CHEST.default_state.id,
                    BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
                let chest = Arc::new(ChestBlockEntity::new(replacement));
                chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
                world.add_block_entity(chest);
            }
        });
        fixture.use_on(FOOT);
        assert_eq!(fixture.world.get_block(&replacement), &Block::CHEST);
        let chest = fixture.world.get_block_entity(&replacement).unwrap();
        let contents = chest.get_inventory().unwrap().get_stack(0);
        assert_eq!(contents.item.id, Item::DIAMOND.id);
        assert_eq!(contents.item_count, 7);
        assert_eq!(fixture.world.get_block_state_id(&partner), partner_state);
        fixture.awake();
        assert_eq!(explosion_packets(&fixture.client.take_packets()), 0);
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_invalid_pair_and_spawn_protection_deny_use() {
    let mut invalid = Fixture::new(Dimension::THE_NETHER);
    invalid.place(&Block::WHITE_BED, &Item::WHITE_BED);
    invalid.world.set_block_state(
        &HEAD,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let foot_state = invalid.world.get_block_state_id(&FOOT);
    invalid.use_on(FOOT);
    assert_eq!(invalid.world.get_block(&HEAD), &Block::CHEST);
    assert_eq!(invalid.world.get_block_state_id(&FOOT), foot_state);
    assert_eq!(explosion_packets(&invalid.client.take_packets()), 0);
    invalid.finish().await;

    let mut protected = Fixture::new(Dimension::OVERWORLD);
    protected.place(&Block::WHITE_BED, &Item::WHITE_BED);
    protected.world.level_time.lock().unwrap().time_of_day = 13_000;
    protected.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.spawn_x = FOOT.0.x;
        info.spawn_z = FOOT.0.z;
        info
    });
    protected
        .server
        .data
        .operator_config
        .write()
        .unwrap()
        .ops
        .push(pumpkin_config::op::Op::new(
            uuid::Uuid::new_v4(),
            "other-operator".to_owned(),
            pumpkin_util::PermissionLvl::Four,
            false,
        ));
    let head_state = protected.world.get_block_state_id(&HEAD);
    protected.use_on(FOOT);
    protected.awake();
    assert_eq!(protected.world.get_block_state_id(&HEAD), head_state);
    assert_eq!(protected.world.get_block(&FOOT), &Block::WHITE_BED);
    protected.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_rebase_bed_removal_retains_completed_capture_and_shutdown_admission() {
    for unsafe_use in [false, true] {
        for stopped in [false, true] {
            let dimension = if unsafe_use {
                Dimension::THE_NETHER
            } else {
                Dimension::OVERWORLD
            };
            let mut fixture = Fixture::new(dimension);
            fixture.place(&Block::WHITE_BED, &Item::WHITE_BED);
            assert!(fixture.world.unload_block_entities(HEAD.chunk_position()));
            if stopped {
                assert!(fixture.world.try_stop_block_entity_mutations());
            }
            if unsafe_use {
                fixture.use_on(FOOT);
            } else {
                fixture
                    .client
                    .player
                    .gamemode
                    .store(pumpkin_util::GameMode::Creative);
                fixture.client.client().handle_player_action(
                    &fixture.client.player,
                    &pumpkin_protocol::java::server::play::SPlayerAction {
                        status: VarInt(0),
                        position: HEAD,
                        face: 1,
                        sequence: VarInt(2),
                    },
                    &fixture.server,
                );
            }
            for position in [HEAD, FOOT] {
                assert_eq!(fixture.world.get_block(&position).is_air(), !stopped);
            }
            assert_eq!(
                explosion_packets(&fixture.client.take_packets()),
                usize::from(unsafe_use && !stopped)
            );
            fixture.finish().await;
        }
    }
}
