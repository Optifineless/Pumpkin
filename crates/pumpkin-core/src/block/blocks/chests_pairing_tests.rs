use super::*;
use crate::{
    block::entities::comparator::ComparatorBlockEntity,
    block::{OnScheduledTickArgs, blocks::redstone::comparator::ComparatorBlock},
    command::CommandSender,
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::block::block_place::BlockPlaceEvent,
    },
    server::{Server, combat_test_support},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    biome::Biome, block_properties::ComparatorLikeProperties, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::generic_container_screen_handler::GenericContainerScreenHandler;
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{Hand, math::vector3::Vector3};
use pumpkin_world::chunk::io::{FileIO, LoadedData};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const LEFT: BlockPos = BlockPos::new(8, 64, 8);
const RIGHT: BlockPos = BlockPos::new(9, 64, 8);
const COMPARATOR: BlockPos = BlockPos::new(8, 64, 9);
const BOUND: Duration = Duration::from_secs(30);

struct Fixture {
    server: Arc<Server>,
    world: Arc<World>,
    player: TestPlayer,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 11.5));
        player.player.get_entity().set_rotation(0.0, 0.0);
        player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn set(&self, position: BlockPos, block: &Block) {
        self.world.set_block_state(
            &position,
            block.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn crouch(&self, crouching: bool) {
        self.player.player.get_entity().set_sneaking(crouching);
        self.player.player.get_entity().pose.store(if crouching {
            EntityPose::Crouching
        } else {
            EntityPose::Standing
        });
    }

    fn hold(&self, item: &'static Item) {
        self.player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, item));
    }

    fn use_on(&self, position: BlockPos, face: BlockDirection) {
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(face as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    fn command(&self, text: &str) {
        let source = CommandSender::Console
            .into_source(&self.server)
            .with_silent();
        assert_eq!(
            self.server
                .command_dispatcher
                .load()
                .execute_input(text, &source),
            Ok(1),
            "{text}"
        );
    }

    fn open(&self, position: BlockPos) -> Option<Arc<dyn Inventory>> {
        self.player.player.close_handled_screen();
        self.crouch(false);
        self.player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::EMPTY.clone());
        self.use_on(position, BlockDirection::Up);
        let screen = self
            .player
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone();
        let screen = screen.lock().unwrap();
        screen
            .as_any()
            .downcast_ref::<GenericContainerScreenHandler>()
            .map(|screen| screen.inventory.clone())
    }

    fn comparator(&self) -> u8 {
        if self.world.get_block(&COMPARATOR) != &Block::COMPARATOR {
            let mut properties = ComparatorLikeProperties::default(&Block::COMPARATOR);
            properties.facing = HorizontalFacing::North;
            self.world.set_block_state(
                &COMPARATOR,
                properties.to_state_id(&Block::COMPARATOR),
                BlockFlags::NOTIFY_LISTENERS,
            );
        }
        ComparatorBlock.on_scheduled_tick(OnScheduledTickArgs {
            world: &self.world,
            block: &Block::COMPARATOR,
            position: &COMPARATOR,
        });
        self.world
            .get_block_entity(&COMPARATOR)
            .unwrap()
            .as_any()
            .downcast_ref::<ComparatorBlockEntity>()
            .unwrap()
            .output_signal
            .load(Ordering::Relaxed)
    }

    fn clear(&self) {
        self.player.player.close_handled_screen();
        self.crouch(false);
        for position in [LEFT, RIGHT, LEFT.up(), RIGHT.up(), COMPARATOR] {
            self.set(position, &Block::AIR);
        }
    }

    async fn reload(&self) {
        let chunk_pos = LEFT.chunk_position();
        tokio::time::timeout(BOUND, self.world.save())
            .await
            .unwrap();
        let chunk = self
            .world
            .level
            .loaded_chunks
            .get(&chunk_pos)
            .unwrap()
            .clone();
        tokio::time::timeout(
            BOUND,
            self.world
                .level
                .chunk_saver
                .save_chunks(&self.world.level.level_folder, vec![(chunk_pos, chunk)]),
        )
        .await
        .unwrap()
        .unwrap();
        self.world.block_entities.remove(&chunk_pos);
        self.world.level.loaded_chunks.remove(&chunk_pos);
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        tokio::time::timeout(
            BOUND,
            self.world.level.chunk_saver.fetch_chunks(
                &self.world.level.level_folder,
                &[chunk_pos],
                send,
            ),
        )
        .await
        .unwrap();
        let LoadedData::Loaded(chunk) = tokio::time::timeout(BOUND, receive.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("saved chest chunk missing")
        };
        self.world.level.loaded_chunks.insert(chunk_pos, chunk);
    }

    async fn finish(self) {
        self.player.player.close_handled_screen();
        tokio::time::timeout(BOUND, self.world.level.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

fn assert_pair(fixture: &Fixture, target: &Block) {
    for (position, half) in [(LEFT, ChestType::Left), (RIGHT, ChestType::Right)] {
        let (block, state) = fixture.world.get_block_and_state_id(&position);
        assert_eq!(
            block, target,
            "both halves must adopt least oxidation and mixed-wax unwaxing"
        );
        let properties = ChestLikeProperties::from_state_id(state);
        assert_eq!(
            properties.r#type, half,
            "placement must connect the two copper variants"
        );
        assert_eq!(properties.facing, HorizontalFacing::North);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_copper_mixed_placement() {
    let fixture = Fixture::new();
    for (first, second, target) in [
        (
            &Item::WAXED_OXIDIZED_COPPER_CHEST,
            &Item::EXPOSED_COPPER_CHEST,
            &Block::EXPOSED_COPPER_CHEST,
        ),
        (
            &Item::WAXED_COPPER_CHEST,
            &Item::OXIDIZED_COPPER_CHEST,
            &Block::COPPER_CHEST,
        ),
        (
            &Item::WAXED_WEATHERED_COPPER_CHEST,
            &Item::WAXED_COPPER_CHEST,
            &Block::WAXED_COPPER_CHEST,
        ),
        (
            &Item::WEATHERED_COPPER_CHEST,
            &Item::EXPOSED_COPPER_CHEST,
            &Block::EXPOSED_COPPER_CHEST,
        ),
    ] {
        for reverse in [false, true] {
            for crouching in [false, true] {
                fixture.clear();
                let (first, second) = if reverse {
                    (second, first)
                } else {
                    (first, second)
                };
                fixture.hold(first);
                fixture.use_on(LEFT.down(), BlockDirection::Up);
                let old = fixture.world.get_block_entity(&LEFT).unwrap();
                let inventory = old.clone().get_inventory().unwrap();
                inventory.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
                fixture.crouch(crouching);
                fixture.hold(second);
                if crouching {
                    fixture.use_on(LEFT, BlockDirection::East);
                } else {
                    fixture.use_on(RIGHT.down(), BlockDirection::Up);
                }
                assert_pair(&fixture, target);
                assert!(
                    Arc::ptr_eq(&old, &fixture.world.get_block_entity(&LEFT).unwrap()),
                    "normalization must retain the old block entity"
                );
                assert_eq!(inventory.get_stack(0).item_count, 7);
                assert!(
                    fixture
                        .player
                        .player
                        .inventory()
                        .get_stack_in_hand(Hand::Right)
                        .is_empty()
                );
                fixture.reload().await;
                assert_pair(&fixture, target);
                assert_eq!(
                    fixture
                        .world
                        .get_block_entity(&LEFT)
                        .unwrap()
                        .get_inventory()
                        .unwrap()
                        .get_stack(0)
                        .item_count,
                    7
                );
            }
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_copper_placement_pending_loot_survives() {
    let fixture = Fixture::new();
    fixture.hold(&Item::WAXED_OXIDIZED_COPPER_CHEST);
    fixture.use_on(LEFT.down(), BlockDirection::Up);
    fixture.command("setblock 8 64 8 waxed_oxidized_copper_chest{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:352L,lock:{\"minecraft:custom_name\":\"vault key\"}} strict");
    let old = fixture.world.get_block_entity(&LEFT).unwrap();
    assert!(old.has_loot_table());
    let lock = old.container_lock();
    fixture.hold(&Item::COPPER_CHEST);
    fixture.use_on(RIGHT.down(), BlockDirection::Up);
    assert_pair(&fixture, &Block::COPPER_CHEST);
    assert!(Arc::ptr_eq(
        &old,
        &fixture.world.get_block_entity(&LEFT).unwrap()
    ));
    assert!(old.has_loot_table());
    assert_eq!(old.container_lock(), lock);
    fixture.reload().await;
    let loaded = fixture.world.get_block_entity(&LEFT).unwrap();
    assert!(loaded.has_loot_table());
    assert_eq!(loaded.container_lock(), lock);
    let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
    loaded.write_internal(&mut nbt);
    assert_eq!(nbt.get_long("LootTableSeed"), Some(352));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_chest_invalid_partner_is_single() {
    let fixture = Fixture::new();
    for partner in [
        "chest[facing=north,type=single]",
        "chest[facing=north,type=left]",
        "chest[facing=south,type=right]",
        "trapped_chest[facing=north,type=right]",
        "copper_chest[facing=north,type=right]",
        "barrel",
    ] {
        fixture.clear();
        fixture.command("setblock 8 64 8 chest[facing=north,type=left] strict");
        fixture.command("setblock 9 64 8 chest[facing=north,type=right] strict");
        fixture.command(&format!("setblock 9 64 8 {partner} strict"));
        let first = fixture
            .world
            .get_block_entity(&LEFT)
            .unwrap()
            .get_inventory()
            .unwrap();
        first.set_stack(0, ItemStack::new(64, &Item::DIAMOND));
        let second = fixture
            .world
            .get_block_entity(&RIGHT)
            .unwrap()
            .get_inventory()
            .unwrap();
        for slot in 0..second.size() {
            second.set_stack(slot, ItemStack::new(64, &Item::STONE));
        }
        assert_eq!(
            fixture.open(LEFT).unwrap().size(),
            27,
            "invalid {partner} must open a single inventory"
        );
        assert_eq!(
            fixture.comparator(),
            1,
            "invalid {partner} must read only the source inventory"
        );
        let data = if partner == "barrel" {
            r#"{Items:[{Slot:0b,id:"minecraft:emerald",count:19}],lock:{"minecraft:custom_name":"vault key"}}"#
        } else {
            r#"{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:352L,lock:{"minecraft:custom_name":"vault key"}}"#
        };
        fixture.command(&format!("setblock 9 64 8 {partner}{data} strict"));
        let unrelated = fixture.world.get_block_entity(&RIGHT).unwrap();
        assert!(unrelated.container_lock().is_some());
        if partner != "barrel" {
            assert!(
                unrelated.has_loot_table(),
                "{partner} must start with pending loot"
            );
        }
        assert_eq!(
            fixture.open(LEFT).unwrap().size(),
            27,
            "an invalid partner lock must not deny opening"
        );
        if partner == "barrel" {
            let stack = unrelated.get_inventory().unwrap().get_stack(0);
            assert_eq!(stack.item, &Item::EMERALD);
            assert_eq!(
                stack.item_count, 19,
                "opening must retain unrelated barrel contents"
            );
        } else {
            assert!(
                unrelated.has_loot_table(),
                "opening a malformed pair must not unpack unrelated loot"
            );
        }
        fixture.set(RIGHT.up(), &Block::STONE);
        assert_eq!(
            fixture.open(LEFT).unwrap().size(),
            27,
            "an invalid partner obstruction must not deny opening"
        );
        assert_eq!(
            fixture.comparator(),
            1,
            "an invalid partner obstruction must not suppress the source signal"
        );
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_ordinary_chest_controls() {
    let fixture = Fixture::new();
    for item in [&Item::CHEST, &Item::TRAPPED_CHEST] {
        fixture.clear();
        for position in [LEFT, RIGHT] {
            fixture.hold(item);
            fixture.use_on(position.down(), BlockDirection::Up);
        }
        let first = fixture
            .world
            .get_block_entity(&LEFT)
            .unwrap()
            .get_inventory()
            .unwrap();
        let second = fixture
            .world
            .get_block_entity(&RIGHT)
            .unwrap()
            .get_inventory()
            .unwrap();
        first.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
        for slot in 0..second.size() {
            second.set_stack(slot, ItemStack::new(64, &Item::STONE));
        }
        for position in [LEFT, RIGHT] {
            let inventory = fixture.open(position).unwrap();
            assert_eq!(inventory.size(), 54);
            assert_eq!(
                inventory.get_stack(0).item,
                &Item::STONE,
                "RIGHT/FIRST must precede LEFT/SECOND from either side"
            );
            assert_eq!(inventory.get_stack(27).item_count, 7);
        }
        assert_eq!(fixture.comparator(), 8);
        fixture.set(RIGHT.up(), &Block::STONE);
        assert!(fixture.open(LEFT).is_none());
        assert_eq!(fixture.comparator(), 0);
    }
    for item in [&Item::CHEST, &Item::TRAPPED_CHEST] {
        fixture.clear();
        fixture.hold(&Item::COPPER_CHEST);
        fixture.use_on(LEFT.down(), BlockDirection::Up);
        fixture.hold(item);
        fixture.use_on(RIGHT.down(), BlockDirection::Up);
        for position in [LEFT, RIGHT] {
            assert_eq!(
                ChestLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
                    .r#type,
                ChestType::Single
            );
            assert_eq!(fixture.open(position).unwrap().size(), 27);
        }
    }
    fixture.finish().await;
}

struct ReplacePlacementPartner {
    world: std::sync::Weak<World>,
    replacement: &'static Block,
    called: AtomicBool,
}
impl EventHandler<BlockPlaceEvent> for ReplacePlacementPartner {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        if event.block_position == RIGHT && !self.called.swap(true, Ordering::Relaxed) {
            self.world.upgrade().unwrap().set_block_state(
                &LEFT,
                self.replacement.default_state.id,
                BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
            );
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g10_copper_placement_revalidates_callback_partner() {
    for replacement in [&Block::COPPER_CHEST, &Block::BARREL] {
        let fixture = Fixture::new();
        fixture.hold(&Item::WAXED_OXIDIZED_COPPER_CHEST);
        fixture.use_on(LEFT.down(), BlockDirection::Up);
        let old = fixture.world.get_block_entity(&LEFT).unwrap();
        old.clone()
            .get_inventory()
            .unwrap()
            .set_stack(0, ItemStack::new(7, &Item::DIAMOND));
        let callback = Arc::new(ReplacePlacementPartner {
            world: Arc::downgrade(&fixture.world),
            replacement,
            called: AtomicBool::new(false),
        });
        fixture
            .server
            .plugin_manager
            .register::<BlockPlaceEvent, _>(callback.clone(), EventPriority::Normal, true);
        fixture.hold(&Item::WAXED_WEATHERED_COPPER_CHEST);
        fixture.use_on(RIGHT.down(), BlockDirection::Up);
        assert!(callback.called.load(Ordering::Relaxed));
        if replacement == &Block::COPPER_CHEST {
            assert_pair(&fixture, &Block::COPPER_CHEST);
            assert!(Arc::ptr_eq(
                &old,
                &fixture.world.get_block_entity(&LEFT).unwrap()
            ));
            assert_eq!(old.get_inventory().unwrap().get_stack(0).item_count, 7);
        } else {
            assert_eq!(
                fixture.world.get_block(&LEFT),
                &Block::BARREL,
                "callback replacement must not be transformed"
            );
            assert_eq!(fixture.open(RIGHT).unwrap().size(), 27);
        }
        fixture.finish().await;
    }
}

#[path = "chests_menu_callback_tests.rs"]
mod menu_callback_tests;
