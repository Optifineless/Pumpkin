use super::*;
use crate::{
    block::{BlockBehaviour, UseWithItemArgs, registry::BlockActionResult},
    item::{ItemBehaviour, ItemMetadata},
    plugin::block::{block_can_build::BlockCanBuildEvent, block_place::BlockPlaceEvent},
};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

struct FailingItem;
impl ItemMetadata for FailingItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::CARROT.id])
    }
}
impl ItemBehaviour for FailingItem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn use_on_block(
        &self,
        _: &mut ItemStack,
        _: &crate::entity::player::Player,
        _: BlockPos,
        _: pumpkin_data::BlockDirection,
        _: Vector3<f32>,
        _: &Block,
        _: &Server,
    ) -> BlockActionResult {
        BlockActionResult::Fail
    }
}
#[pumpkin_block("minecraft:stone")]
struct FailingBlock;
impl BlockBehaviour for FailingBlock {
    fn use_with_item(&self, _: UseWithItemArgs<'_>) -> BlockActionResult {
        BlockActionResult::Fail
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_explicit_block_and_item_failure_deny_fallback() {
    for item_failure in [false, true] {
        let scenario =
            Scenario::with_registry(&Item::CARROT, Hand::Right, &Block::STONE, |server| {
                if item_failure {
                    Arc::get_mut(&mut server.item_registry)
                        .unwrap()
                        .register(FailingItem);
                } else {
                    Arc::get_mut(&mut server.block_registry)
                        .unwrap()
                        .register(FailingBlock);
                }
            });
        let events = register_consume_callback(&scenario, Edit::None);
        scenario.use_block();
        assert_eq!(scenario.active(), None);
        scenario.tick(64);
        scenario.assert_stock_and_food(2, 10);
        assert_eq!(&*events.lock().unwrap(), &["interact"]);
        scenario.finish().await;
    }
}

struct CancelInteraction;
impl EventHandler<PlayerInteractEvent> for CancelInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_interaction_cancellation_denies_fallback() {
    let scenario = Scenario::new(&Item::CARROT, Hand::Left, &Block::STONE);
    scenario
        .server
        .plugin_manager
        .register::<PlayerInteractEvent, _>(
            Arc::new(CancelInteraction),
            EventPriority::Normal,
            true,
        );
    scenario.use_block();
    assert_eq!(scenario.active(), None);
    scenario.tick(64);
    scenario.assert_stock_and_food(2, 10);
    scenario.finish().await;
}

#[derive(Clone, Copy)]
enum PlacementEdit {
    CancelBuild,
    Unbuildable,
    CancelPlace,
    Replace,
    Select,
    Occupy,
}
struct PlacementCallback {
    edit: PlacementEdit,
    source: usize,
    events: Arc<Mutex<Vec<&'static str>>>,
    calls: AtomicUsize,
}
impl EventHandler<BlockCanBuildEvent> for PlacementCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockCanBuildEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push("build");
        assert!(event.buildable);
        assert_eq!(event.block, &Block::FARMLAND);
        assert!(
            event.block_to_build == &Block::CARROTS || event.block_to_build == &Block::POTATOES
        );
        match self.edit {
            PlacementEdit::CancelBuild => event.cancelled = true,
            PlacementEdit::Unbuildable => event.buildable = false,
            _ => {}
        }
        Box::pin(async {})
    }
}
impl EventHandler<BlockPlaceEvent> for PlacementCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        self.events.lock().unwrap().push("place");
        self.calls.fetch_add(1, SeqCst);
        let inventory = event.player.inventory();
        match self.edit {
            PlacementEdit::CancelPlace => event.cancelled = true,
            PlacementEdit::Replace => {
                let stack = inventory.get_stack(self.source);
                inventory.set_stack(self.source, ItemStack::new(stack.item_count, stack.item));
            }
            PlacementEdit::Select => {
                inventory.set_stack(1, ItemStack::new(2, &Item::BREAD));
                inventory.set_selected_slot(1);
            }
            PlacementEdit::Occupy => {
                event.player.world().set_block_state(
                    &ABOVE,
                    Block::OBSIDIAN.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
            }
            PlacementEdit::CancelBuild | PlacementEdit::Unbuildable => {}
        }
        Box::pin(async {})
    }
}
fn register_placement_callback(
    scenario: &Scenario,
    edit: PlacementEdit,
    events: Arc<Mutex<Vec<&'static str>>>,
) -> Arc<PlacementCallback> {
    let callback = Arc::new(PlacementCallback {
        edit,
        source: scenario.source,
        events,
        calls: AtomicUsize::new(0),
    });
    scenario
        .server
        .plugin_manager
        .register::<BlockCanBuildEvent, _>(callback.clone(), EventPriority::Normal, true);
    scenario
        .server
        .plugin_manager
        .register::<BlockPlaceEvent, _>(callback.clone(), EventPriority::Normal, true);
    callback
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_cancelled_placement_and_can_build_do_not_eat() {
    for edit in [
        PlacementEdit::CancelBuild,
        PlacementEdit::Unbuildable,
        PlacementEdit::CancelPlace,
    ] {
        for hand in [Hand::Right, Hand::Left] {
            let scenario = Scenario::new(&Item::POTATO, hand, &Block::FARMLAND);
            let events = register_consume_callback(&scenario, Edit::None);
            register_placement_callback(&scenario, edit, events.clone());
            scenario.use_block();
            assert_eq!(scenario.active(), None);
            scenario.tick(64);
            scenario.assert_stock_and_food(2, 10);
            assert_eq!(scenario.world.get_block(&ABOVE), &Block::AIR);
            let expected: &[&str] = if matches!(edit, PlacementEdit::CancelPlace) {
                &["interact", "build", "place"]
            } else {
                &["interact", "build"]
            };
            assert_eq!(&*events.lock().unwrap(), expected);
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_placement_callback_replacement_and_target_edit_do_not_eat() {
    for edit in [PlacementEdit::Replace, PlacementEdit::Occupy] {
        for hand in [Hand::Right, Hand::Left] {
            let scenario = Scenario::new(&Item::CARROT, hand, &Block::FARMLAND);
            let events = register_consume_callback(&scenario, Edit::None);
            let callback = register_placement_callback(&scenario, edit, events.clone());
            scenario.use_block();
            assert_eq!(callback.calls.load(SeqCst), 1);
            assert_eq!(scenario.active(), None);
            scenario.tick(64);
            assert_eq!(
                scenario
                    .fixture
                    .player
                    .inventory()
                    .get_stack(scenario.source)
                    .item_count,
                2
            );
            assert_eq!(scenario.fixture.player.hunger_manager.level.load(), 10);
            assert_eq!(&*events.lock().unwrap(), &["interact", "build", "place"]);
            if matches!(edit, PlacementEdit::Replace) {
                assert_ne!(
                    scenario
                        .fixture
                        .player
                        .inventory()
                        .get_stack(scenario.source)
                        .uid,
                    scenario.original.uid
                );
                assert_eq!(scenario.world.get_block(&ABOVE), &Block::AIR);
            } else {
                assert_eq!(scenario.world.get_block(&ABOVE), &Block::OBSIDIAN);
            }
            scenario.finish().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_planting_selection_change_pays_pinned_source_once() {
    for hand in [Hand::Right, Hand::Left] {
        let scenario = Scenario::new(&Item::CARROT, hand, &Block::FARMLAND);
        let events = register_consume_callback(&scenario, Edit::None);
        register_placement_callback(&scenario, PlacementEdit::Select, events.clone());
        scenario.use_block();
        assert_eq!(scenario.active(), None);
        scenario.tick(64);
        scenario.assert_stock_and_food(1, 10);
        assert_eq!(scenario.world.get_block(&ABOVE), &Block::CARROTS);
        assert_eq!(scenario.fixture.player.inventory().get_selected_slot(), 1);
        assert_eq!(
            scenario.fixture.player.inventory().get_stack(1).item_count,
            2
        );
        assert_eq!(&*events.lock().unwrap(), &["interact", "build", "place"]);
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g18_spawn_protection_does_not_fall_back_to_eating() {
    let scenario = Scenario::new(&Item::CARROT, Hand::Right, &Block::STONE);
    let mut info = (**scenario.server.level_info.load()).clone();
    info.spawn_x = CLICKED.0.x;
    info.spawn_z = CLICKED.0.z;
    scenario.server.level_info.store(Arc::new(info));
    scenario
        .server
        .data
        .operator_config
        .write()
        .unwrap()
        .ops
        .push(pumpkin_config::op::Op {
            uuid: uuid::Uuid::new_v4(),
            ..Default::default()
        });
    assert!(
        scenario
            .world
            .is_in_spawn_protection(&scenario.fixture.player, &CLICKED)
    );
    let events = register_consume_callback(&scenario, Edit::None);
    scenario.use_block();
    assert_eq!(scenario.active(), None);
    scenario.tick(64);
    scenario.assert_stock_and_food(2, 10);
    assert!(events.lock().unwrap().is_empty());
    scenario.finish().await;
}
