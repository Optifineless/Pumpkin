use crate::{
    entity::EntityBase,
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority},
    server::{Server, combat_test_support},
    world::{
        World,
        spawn_test_support::{proto, publish},
    },
};
use pumpkin_data::{
    Block, BlockDirection,
    biome::Biome,
    block_properties::DecoratedPotLikeProperties,
    data_component_impl::{DataComponentImpl, PotDecoration, PotDecorationsImpl},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    java::server::play::{SPlayerAction, SUseItemOn},
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};
use pumpkin_world::world::BlockFlags;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

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
        publish(&world, proto(&Biome::PLAINS, &Block::AIR));
        let player = TestPlayer::new(&world);
        Self {
            server,
            world,
            player,
            _directory: directory,
        }
    }

    fn approach(&self, block: BlockPos) {
        self.player.player.get_entity().set_pos(Vector3::new(
            f64::from(block.0.x) + 0.5,
            f64::from(block.0.y),
            f64::from(block.0.z) + 2.5,
        ));
    }

    fn set_block(&self, position: BlockPos, block: &Block) {
        self.world.set_block_state(
            &position,
            block.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
    }

    fn give(&self, stack: ItemStack) {
        self.player.player.inventory().set_stack(0, stack);
    }

    fn use_on(&self, position: BlockPos) {
        self.approach(position);
        self.player
            .client()
            .handle_use_item_on(
                &self.player.player,
                &SUseItemOn {
                    hand: VarInt(0),
                    position,
                    face: VarInt(BlockDirection::Up as i32),
                    cursor_pos: Vector3::new(0.5, 0.5, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    fn place_pot(&self, support: BlockPos, decorations: PotDecorationsImpl) -> BlockPos {
        let pot = support.up();
        self.set_block(support, &Block::STONE);
        self.set_block(pot, &Block::AIR);
        let mut stack = ItemStack::new(1, &Item::DECORATED_POT);
        stack.set_data_component(decorations);
        self.give(stack);
        self.use_on(support);
        assert_eq!(self.world.get_block(&pot), &Block::DECORATED_POT);
        assert!(self.world.get_block_entity(&pot).is_some());
        pot
    }

    fn insert(&self, pot: BlockPos, item: &'static Item) {
        self.give(ItemStack::new(1, item));
        self.use_on(pot);
        assert!(self.world.get_block_entity(&pot).is_some());
        assert!(self.player.player.inventory().held_item().is_empty());
    }

    fn dig(&self, position: BlockPos) {
        self.approach(position);
        self.player.client().handle_player_action(
            &self.player.player,
            &SPlayerAction {
                // SPlayerAction.Status::FinishedDigging.
                status: VarInt(2),
                position,
                face: 1,
                sequence: VarInt(1),
            },
            &self.server,
        );
    }
}

fn decoration(item: &'static Item, count: i32) -> PotDecoration {
    PotDecoration {
        item,
        count,
        patch: Vec::new(),
        raw_components: Vec::new(),
    }
}

fn decorated(items: [&'static Item; 4]) -> PotDecorationsImpl {
    PotDecorationsImpl {
        decorations: items.map(|item| Some(decoration(item, 1))),
    }
}

fn item_count(world: &World, item: &Item) -> u32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|entity| {
            let stack = entity
                .get_item_stack()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if stack.item.id == item.id {
                u32::from(stack.item_count)
            } else {
                0
            }
        })
        .sum()
}

fn take_pot_drop(world: &World) -> Option<ItemStack> {
    for entity in world.entities.load().iter() {
        let Some(item_entity) = entity.get_item_entity() else {
            continue;
        };
        let mut stack = item_entity
            .get_item_stack()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stack.item.id == Item::DECORATED_POT.id && !stack.is_empty() {
            return Some(std::mem::replace(&mut *stack, ItemStack::EMPTY.clone()));
        }
    }
    None
}

struct CaptureDropState(Arc<AtomicBool>);

impl EventHandler<crate::plugin::block::block_drop_item::BlockDropItemEvent> for CaptureDropState {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::block::block_drop_item::BlockDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        let state = event.world.get_block_state(&event.block_pos);
        let properties = DecoratedPotLikeProperties::from_state_id(state.id);
        self.0.store(properties.cracked, Relaxed);
        Box::pin(async {})
    }
}

struct CancelBreak;

impl EventHandler<crate::plugin::api::events::block::block_break::BlockBreakEvent> for CancelBreak {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::block::block_break::BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_intact_loot_scatter_and_reuse_do_not_create_materials() {
    let fixture = Fixture::new();
    let initial = decorated([
        &Item::ANGLER_POTTERY_SHERD,
        &Item::ARCHER_POTTERY_SHERD,
        &Item::ARMS_UP_POTTERY_SHERD,
        &Item::BLADE_POTTERY_SHERD,
    ]);
    let first = fixture.place_pot(BlockPos::new(8, 64, 8), initial.clone());
    fixture.insert(first, &Item::DIAMOND);
    fixture.give(ItemStack::EMPTY.clone());
    fixture.dig(first);

    assert_eq!(fixture.world.get_block(&first), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 1);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 1);
    let returned = take_pot_drop(&fixture.world).expect("intact pot item");
    assert_eq!(
        returned.get_data_component::<PotDecorationsImpl>(),
        Some(&initial)
    );
    assert!(
        returned
            .get_data_component::<pumpkin_data::data_component_impl::ContainerImpl>()
            .is_some_and(|contents| contents.items.is_empty()),
        "the returned pot has no duplicate item in its default container component"
    );

    // Reuse the actual ground drop through Java block placement and add new contents.
    let second = BlockPos::new(10, 65, 8);
    fixture.set_block(BlockPos::new(10, 64, 8), &Block::STONE);
    fixture.set_block(second, &Block::AIR);
    fixture.give(returned);
    fixture.use_on(BlockPos::new(10, 64, 8));
    assert_eq!(fixture.world.get_block(&second), &Block::DECORATED_POT);
    fixture.insert(second, &Item::DIAMOND);
    fixture.give(ItemStack::EMPTY.clone());
    fixture.dig(second);

    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 1);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 2);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_eligible_tool_uses_cracked_mixed_decoration_loot() {
    let fixture = Fixture::new();
    let position = fixture.place_pot(
        BlockPos::new(8, 64, 8),
        decorated([
            &Item::BRICK,
            &Item::ANGLER_POTTERY_SHERD,
            &Item::BRICK,
            &Item::ARCHER_POTTERY_SHERD,
        ]),
    );
    let states = Arc::new(AtomicBool::new(false));
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::block::block_drop_item::BlockDropItemEvent, _>(
            Arc::new(CaptureDropState(states.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert!(
        states.load(Relaxed),
        "the plugin drop event observes the cracked state during loot"
    );
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 2);
    assert_eq!(item_count(&fixture.world, &Item::ANGLER_POTTERY_SHERD), 1);
    assert_eq!(item_count(&fixture.world, &Item::ARCHER_POTTERY_SHERD), 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_shattering_protection_keeps_intact_loot() {
    let fixture = Fixture::new();
    let decorations = decorated([
        &Item::ANGLER_POTTERY_SHERD,
        &Item::ARCHER_POTTERY_SHERD,
        &Item::ARMS_UP_POTTERY_SHERD,
        &Item::BLADE_POTTERY_SHERD,
    ]);
    let position = fixture.place_pot(BlockPos::new(8, 64, 8), decorations.clone());
    let mut pickaxe = ItemStack::new(1, &Item::DIAMOND_PICKAXE);
    pickaxe.add_enchantment(&pumpkin_data::Enchantment::SILK_TOUCH, 1);
    fixture.give(pickaxe);
    fixture.dig(position);

    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 1);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    assert_eq!(item_count(&fixture.world, &Item::ANGLER_POTTERY_SHERD), 0);
    assert_eq!(
        take_pot_drop(&fixture.world)
            .expect("protected intact pot")
            .get_data_component::<PotDecorationsImpl>(),
        Some(&decorations)
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_creative_break_keeps_no_drop_rule() {
    let fixture = Fixture::new();
    let position = fixture.place_pot(
        BlockPos::new(8, 64, 8),
        decorated([
            &Item::ANGLER_POTTERY_SHERD,
            &Item::ARCHER_POTTERY_SHERD,
            &Item::ARMS_UP_POTTERY_SHERD,
            &Item::BLADE_POTTERY_SHERD,
        ]),
    );
    fixture.player.player.gamemode.store(GameMode::Creative);
    fixture.give(ItemStack::EMPTY.clone());
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::AIR);
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    assert_eq!(item_count(&fixture.world, &Item::ANGLER_POTTERY_SHERD), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_cancelled_break_preserves_pot_and_emits_no_loot() {
    let fixture = Fixture::new();
    let position = fixture.place_pot(
        BlockPos::new(8, 64, 8),
        decorated([
            &Item::ANGLER_POTTERY_SHERD,
            &Item::ARCHER_POTTERY_SHERD,
            &Item::ARMS_UP_POTTERY_SHERD,
            &Item::BLADE_POTTERY_SHERD,
        ]),
    );
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::block::block_break::BlockBreakEvent, _>(
            Arc::new(CancelBreak),
            EventPriority::Normal,
            true,
        );
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);
    assert!(
        !DecoratedPotLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
            .cracked
    );
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g03_spawn_protected_break_preserves_pot_and_emits_no_loot() {
    let fixture = Fixture::new();
    let support = BlockPos::new(8, 64, 8);
    let position = fixture.place_pot(
        support,
        decorated([
            &Item::ANGLER_POTTERY_SHERD,
            &Item::ARCHER_POTTERY_SHERD,
            &Item::ARMS_UP_POTTERY_SHERD,
            &Item::BLADE_POTTERY_SHERD,
        ]),
    );
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.spawn_x = support.0.x;
        next.spawn_y = support.0.y;
        next.spawn_z = support.0.z;
        next
    });
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ops
        .push(pumpkin_config::op::Op::new(
            uuid::Uuid::new_v4(),
            "b1019-g03-spawn-control".to_owned(),
            PermissionLvl::Zero,
            false,
        ));
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    fixture.dig(position);

    assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);
    assert!(
        !DecoratedPotLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
            .cracked
    );
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 0);
    fixture.server.shutdown().await;
}

#[test]
fn b1019_g03_legacy_sherds_load_and_save_without_changing_the_old_field() {
    use crate::block::entities::{BlockEntity, decorated_pot::DecoratedPotBlockEntity};
    use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

    let sherds = vec![
        NbtTag::String("minecraft:brick".into()),
        NbtTag::String("minecraft:angler_pottery_sherd".into()),
        NbtTag::String("minecraft:archer_pottery_sherd".into()),
        NbtTag::String("minecraft:brick".into()),
    ];
    let mut old_save = NbtCompound::new();
    old_save.put_list("sherds", sherds.clone());
    let loaded = DecoratedPotBlockEntity::from_nbt(&old_save, BlockPos::new(0, 64, 0));
    let mut saved = NbtCompound::new();
    loaded.write_nbt(&mut saved);

    assert_eq!(saved.get_list("sherds"), Some(sherds.as_slice()));
    let encoded = saved.get("PotDecorations").unwrap();
    let converted = PotDecorationsImpl::read_data(encoded).unwrap();
    assert_eq!(
        converted.decorations[0].as_ref().unwrap().item,
        &Item::BRICK
    );
    assert_eq!(
        converted.decorations[1].as_ref().unwrap().item,
        &Item::ANGLER_POTTERY_SHERD
    );
    assert_eq!(
        converted.decorations[2].as_ref().unwrap().item,
        &Item::ARCHER_POTTERY_SHERD
    );
    assert_eq!(
        converted.decorations[3].as_ref().unwrap().item,
        &Item::BRICK
    );
}

#[path = "decorated_pot_crafting_tests.rs"]
mod crafting_tests;
