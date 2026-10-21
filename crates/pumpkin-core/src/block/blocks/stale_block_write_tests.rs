use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use pumpkin_data::{
    Block, BlockStateId,
    biome::Biome,
    block_properties::{
        ChestLikeProperties, ChestType, DoubleBlockHalf, HorizontalFacing, OakDoorLikeProperties,
    },
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, Status},
};
use pumpkin_util::{Hand, math::position::BlockPos};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::RandomTickArgs,
    entity::{EntityBase, player::Player},
    net::{ClientPlatform, java::combat_test_support::TestPlayer},
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};

const TARGET: BlockPos = BlockPos::new(8, 64, 8);

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
        publish(&world, proto(&Biome::PLAINS, &Block::STONE));
        let player = TestPlayer::new(&world);
        player.player.advancements.try_lock().unwrap().player = Arc::downgrade(&player.player);
        player
            .player
            .inventory()
            .set_stack_in_hand(Hand::Right, ItemStack::new(1, &Item::IRON_PICKAXE));
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn put(&self, position: BlockPos, state: BlockStateId) {
        self.world.set_block_state(
            &position,
            state,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn random_tick(&self, position: BlockPos) {
        let block = self.world.get_block(&position);
        self.world
            .block_registry
            .get_pumpkin_block(block.id)
            .unwrap()
            .random_tick(RandomTickArgs {
                world: &self.world,
                block,
                position: &position,
            });
    }

    fn mine_before_write(&self, position: BlockPos) -> Arc<AtomicBool> {
        let called = Arc::new(AtomicBool::new(false));
        let observed = called.clone();
        let player = self.player.player.clone();
        let server = self.server.clone();
        let world = Arc::downgrade(&self.world);
        arm_before_write(
            &self.world,
            position,
            Arc::new(move || {
                assert!(!observed.swap(true, Ordering::Relaxed));
                destroy_with_java(&player, &server, position);
                assert_eq!(
                    world.upgrade().unwrap().get_block(&position),
                    &Block::AIR,
                    "the competing real player break must finish before the tick resumes"
                );
            }),
        );
        called
    }

    fn tick_until_write(&self, position: BlockPos, called: &AtomicBool) {
        for _ in 0..10_000 {
            self.random_tick(position);
            if called.load(Ordering::Relaxed) {
                return;
            }
        }
        panic!("random tick did not reach its block-write seam");
    }

    fn item_count(&self, item: &Item) -> u32 {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|entity| {
                let stack = entity.get_item_stack().lock().unwrap();
                if stack.item == item {
                    u32::from(stack.item_count)
                } else {
                    0
                }
            })
            .sum()
    }

    fn stock_chest(&self, position: BlockPos, diamonds: u8) {
        self.world
            .get_block_entity(&position)
            .unwrap()
            .get_inventory()
            .unwrap()
            .set_stack(0, ItemStack::new(diamonds, &Item::DIAMOND));
    }
}

// Ignore writes to other positions, including the first half of a paired commit.
// Both the old unconditional writer and the conditional writer expose this seam.
fn arm_before_write(world: &Arc<World>, target: BlockPos, callback: Arc<dyn Fn() + Send + Sync>) {
    let weak = Arc::downgrade(world);
    *world.persistence_test_hooks.state_write.lock().unwrap() = Some(Arc::new(move |position| {
        if position == target {
            callback();
        } else {
            arm_before_write(&weak.upgrade().unwrap(), target, callback.clone());
        }
    }));
}

fn destroy_with_java(player: &Arc<Player>, server: &Server, position: BlockPos) {
    player.get_entity().set_pos(position.to_centered_f64());
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java player required")
    };
    client.handle_player_action(
        player,
        &SPlayerAction {
            status: VarInt(Status::FinishedDigging as i32),
            position,
            face: 1,
            sequence: VarInt(1),
        },
        server,
    );
}

fn place_double_chest(fixture: &Fixture) -> BlockPos {
    let right = TARGET.offset(pumpkin_util::math::vector3::Vector3::new(1, 0, 0));
    for (position, half) in [(TARGET, ChestType::Left), (right, ChestType::Right)] {
        let mut properties = ChestLikeProperties::default(&Block::COPPER_CHEST);
        properties.facing = HorizontalFacing::North;
        properties.r#type = half;
        fixture.put(position, properties.to_state_id(&Block::COPPER_CHEST));
    }
    right
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_single_chest_player_break_drops_once_and_stays_air() {
    let fixture = Fixture::new();
    fixture.put(TARGET, Block::COPPER_CHEST.default_state.id);
    fixture.stock_chest(TARGET, 5);
    let called = fixture.mine_before_write(TARGET);

    fixture.tick_until_write(TARGET, &called);

    assert_eq!(fixture.item_count(&Item::COPPER_CHEST), 1);
    assert_eq!(fixture.item_count(&Item::DIAMOND), 5);
    assert_eq!(
        fixture.world.get_block(&TARGET),
        &Block::AIR,
        "stale oxidation resurrected the mined single copper chest"
    );
    assert!(fixture.world.get_block_entity(&TARGET).is_none());
    fixture.world.level.shutdown().await.unwrap();
}

async fn double_chest_break(right_half: bool) {
    let fixture = Fixture::new();
    let right = place_double_chest(&fixture);
    fixture.stock_chest(TARGET, 5);
    fixture.stock_chest(right, 7);
    let (mined, survivor, diamonds) = if right_half {
        (right, TARGET, 7)
    } else {
        (TARGET, right, 5)
    };
    let called = fixture.mine_before_write(mined);

    fixture.tick_until_write(TARGET, &called);

    assert_eq!(fixture.item_count(&Item::COPPER_CHEST), 1);
    assert_eq!(fixture.item_count(&Item::DIAMOND), diamonds);
    assert_eq!(
        fixture.world.get_block(&mined),
        &Block::AIR,
        "stale oxidation resurrected a mined double-chest half"
    );
    assert!(fixture.world.get_block_entity(&mined).is_none());
    assert_eq!(fixture.world.get_block(&survivor), &Block::COPPER_CHEST);
    let stack = fixture
        .world
        .get_block_entity(&survivor)
        .unwrap()
        .get_inventory()
        .unwrap()
        .get_stack(0);
    assert_eq!(stack.item, &Item::DIAMOND);
    assert_eq!(u32::from(stack.item_count) + diamonds, 12);
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_double_chest_left_player_break_drops_once_and_stays_air() {
    double_chest_break(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_double_chest_right_player_break_drops_once_and_stays_air() {
    double_chest_break(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_trapdoor_player_break_drops_once_and_stays_air() {
    let fixture = Fixture::new();
    fixture.put(TARGET, Block::COPPER_TRAPDOOR.default_state.id);
    let called = fixture.mine_before_write(TARGET);

    fixture.tick_until_write(TARGET, &called);

    assert_eq!(fixture.item_count(&Item::COPPER_TRAPDOOR), 1);
    assert_eq!(
        fixture.world.get_block(&TARGET),
        &Block::AIR,
        "stale oxidation resurrected the mined copper trapdoor"
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_door_player_break_leaves_both_halves_air() {
    for mined in [TARGET, TARGET.up()] {
        let fixture = Fixture::new();
        for (position, half) in [
            (TARGET, DoubleBlockHalf::Lower),
            (TARGET.up(), DoubleBlockHalf::Upper),
        ] {
            let mut properties = OakDoorLikeProperties::default(&Block::COPPER_DOOR);
            properties.half = half;
            fixture.put(position, properties.to_state_id(&Block::COPPER_DOOR));
        }
        let called = fixture.mine_before_write(mined);

        fixture.tick_until_write(TARGET, &called);

        // Existing upper-half teardown skips the lower half's loot before the player broken hook.
        assert_eq!(
            fixture.item_count(&Item::COPPER_DOOR),
            u32::from(mined == TARGET)
        );
        assert_eq!(
            fixture.world.get_block(&TARGET),
            &Block::AIR,
            "stale oxidation resurrected the mined copper door's lower half"
        );
        assert_eq!(fixture.world.get_block(&TARGET.up()), &Block::AIR);
        fixture.world.level.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_growth_wheat_player_harvest_drops_once_and_stays_air() {
    let fixture = Fixture::new();
    fixture.put(TARGET.down(), Block::FARMLAND.default_state.id);
    fixture.put(TARGET, Block::WHEAT.default_state.id);
    fixture.world.set_sky_light_level(&TARGET, 15);
    fixture.world.set_block_light_level(&TARGET, 15);
    let called = fixture.mine_before_write(TARGET);

    fixture.tick_until_write(TARGET, &called);

    assert_eq!(fixture.item_count(&Item::WHEAT_SEEDS), 1);
    assert_eq!(
        fixture.world.get_block(&TARGET),
        &Block::AIR,
        "stale crop growth resurrected the harvested wheat"
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_oxidation_uncontested_single_and_double_chests_advance_once() {
    for double in [false, true] {
        let fixture = Fixture::new();
        let positions = if double {
            vec![TARGET, place_double_chest(&fixture)]
        } else {
            fixture.put(TARGET, Block::COPPER_CHEST.default_state.id);
            vec![TARGET]
        };
        for (index, position) in positions.iter().enumerate() {
            fixture.stock_chest(*position, index as u8 + 3);
        }
        for _ in 0..10_000 {
            fixture.random_tick(TARGET);
            if fixture.world.get_block(&TARGET) != &Block::COPPER_CHEST {
                break;
            }
        }

        for (index, position) in positions.into_iter().enumerate() {
            assert_eq!(
                fixture.world.get_block(&position),
                &Block::EXPOSED_COPPER_CHEST
            );
            let properties =
                ChestLikeProperties::from_state_id(fixture.world.get_block_state_id(&position));
            assert_eq!(
                properties.r#type,
                if !double {
                    ChestType::Single
                } else if index == 0 {
                    ChestType::Left
                } else {
                    ChestType::Right
                }
            );
            let stack = fixture
                .world
                .get_block_entity(&position)
                .unwrap()
                .get_inventory()
                .unwrap()
                .get_stack(0);
            assert_eq!(stack.item, &Item::DIAMOND);
            assert_eq!(stack.item_count, index as u8 + 3);
        }
        assert_eq!(fixture.item_count(&Item::COPPER_CHEST), 0);
        assert_eq!(fixture.item_count(&Item::DIAMOND), 0);
        fixture.world.level.shutdown().await.unwrap();
    }
}
