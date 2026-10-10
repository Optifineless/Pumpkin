//! Listener-free fixtures exercise the registered block through `JavaClient.handle_use_item_on`.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    block::{BlockHitResult, registry::BlockActionResult},
    entity::{EntityBase, item::ItemEntity},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        block::cauldron_level_change::{CauldronChangeReason, CauldronLevelChangeEvent},
        world::generic_game::GenericGameEvent,
    },
    server::{Server, combat_test_support},
    world::{World, spawn_test_support},
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    biome::Biome,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, ContainerImpl, CustomNameImpl, DyedColorImpl,
        EquipmentSlot,
    },
    dye_color::DyeColor,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{codec::var_int::VarInt, java::server::play::SUseItemOn};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
    text::TextComponent,
};
use pumpkin_world::world::BlockFlags;

pub(super) const POS: BlockPos = BlockPos::new(8, 64, 8);
pub(super) type Change = (BlockStateId, i32, i32, CauldronChangeReason);

#[derive(Default)]
pub(super) struct EventLog {
    pub cancel: AtomicBool,
    pub changes: Mutex<Vec<Change>>,
    pub game_events: Mutex<Vec<String>>,
}

impl EventHandler<CauldronLevelChangeEvent> for EventLog {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut CauldronLevelChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(event.block_pos, POS);
            assert!(event.entity.is_some());
            self.changes.lock().unwrap().push((
                event.world.get_block_state_id(&POS),
                event.old_level,
                event.new_level,
                event.reason,
            ));
            event.cancelled = self.cancel.load(Ordering::Relaxed);
        })
    }
}

impl EventHandler<GenericGameEvent> for EventLog {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.game_events
                .lock()
                .unwrap()
                .push(event.event_key.clone());
        })
    }
}

pub(super) struct Fixture {
    _dir: tempfile::TempDir,
    pub server: Arc<Server>,
    pub world: Arc<World>,
    pub client: TestPlayer,
    pub events: Arc<EventLog>,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut server = combat_test_support::server(dir.path());
        Arc::get_mut(&mut server)
            .unwrap()
            .basic_config
            .spawn_protection = 0;
        let world = combat_test_support::world(&server, dir.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        spawn_test_support::publish(
            &world,
            spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
        );
        let client = TestPlayer::new(&world);
        client
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 6.5));
        let events = Arc::new(EventLog::default());
        server
            .plugin_manager
            .register::<CauldronLevelChangeEvent, _>(events.clone(), EventPriority::Normal, true);
        server.plugin_manager.register::<GenericGameEvent, _>(
            events.clone(),
            EventPriority::Normal,
            true,
        );
        Self {
            _dir: dir,
            server,
            world,
            client,
            events,
        }
    }

    pub fn reset(&mut self, state: BlockStateId, hand: Hand, stack: ItemStack) {
        let inventory = self.client.player.inventory();
        for slot in 0..inventory.size() {
            inventory.set_stack(slot, ItemStack::EMPTY.clone());
        }
        inventory.set_stack_in_hand(hand, stack);
        inventory.set_stack_in_hand(other_hand(hand), ItemStack::new(1, &Item::DIAMOND_SWORD));
        self.client.player.stats.lock().unwrap().stats.clear();
        self.put(POS.up(), Block::AIR.default_state.id);
        self.put(POS, state);
        self.clear_effects();
    }

    pub fn clear_effects(&mut self) {
        self.events.changes.lock().unwrap().clear();
        self.events.game_events.lock().unwrap().clear();
        self.client.take_packets();
    }

    pub fn put(&self, pos: BlockPos, state: BlockStateId) {
        self.world
            .set_block_state(&pos, state, BlockFlags::NOTIFY_ALL);
    }

    pub fn use_top(&self, hand: Hand) {
        self.client
            .client()
            .handle_use_item_on(
                &self.client.player,
                &SUseItemOn {
                    hand: VarInt(i32::from(!matches!(hand, Hand::Right))),
                    position: POS,
                    face: VarInt(1),
                    cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                    inside_block: false,
                    is_against_world_border: false,
                    sequence: VarInt(1),
                },
                &self.server,
            )
            .unwrap();
    }

    /// Exposes the registered result in refusal cases, where `JavaClient` otherwise returns ().
    pub fn block_result(&self, hand: Hand) -> BlockActionResult {
        let mut stack = self.client.player.inventory().get_stack_in_hand(hand);
        let before = stack.clone();
        let result = self.server.block_registry.use_with_item(
            self.world.get_block(&POS),
            &self.client.player,
            &POS,
            &BlockHitResult {
                face: &BlockDirection::Up,
                cursor_pos: &Vector3::new(0.5, 1.0, 0.5),
            },
            &mut stack,
            if matches!(hand, Hand::Right) {
                &EquipmentSlot::MAIN_HAND
            } else {
                &EquipmentSlot::OFF_HAND
            },
            &self.server,
            &self.world,
        );
        assert!(
            stack.are_equal(&before),
            "refused block use must preserve its mutable input"
        );
        result
    }

    pub fn inventory(&self) -> Vec<ItemStack> {
        let inventory = self.client.player.inventory();
        (0..inventory.size())
            .map(|slot| inventory.get_stack(slot))
            .collect()
    }

    pub fn count(&self, item: &Item) -> u32 {
        self.inventory()
            .iter()
            .filter(|stack| stack.get_item() == item)
            .map(|stack| u32::from(stack.item_count))
            .sum()
    }

    pub fn sound_count(&mut self) -> usize {
        self.client
            .take_packets()
            .iter()
            .filter(|packet| {
                VarInt::decode(&mut packet.as_ref()).unwrap().0
                    == pumpkin_data::packet::clientbound::play::SOUND.0
            })
            .count()
    }

    pub fn drops(&self) -> Vec<ItemStack> {
        self.world
            .entities
            .load()
            .iter()
            .filter_map(|entity| {
                entity
                    .cast_any()
                    .downcast_ref::<ItemEntity>()
                    .map(|item| item.get_item_stack().lock().unwrap().clone())
            })
            .collect()
    }

    pub fn assert_unchanged(&mut self, state: BlockStateId, inventory: &[ItemStack]) {
        assert_eq!(self.world.get_block_state_id(&POS), state);
        assert!(
            self.inventory()
                .iter()
                .zip(inventory)
                .all(|(a, b)| a.are_equal(b))
        );
        assert!(self.client.player.stats.lock().unwrap().stats.is_empty());
        assert!(self.events.game_events.lock().unwrap().is_empty());
        assert_eq!(self.sound_count(), 0);
        assert!(self.drops().is_empty());
    }
}

pub(super) fn other_hand(hand: Hand) -> Hand {
    if matches!(hand, Hand::Right) {
        Hand::Left
    } else {
        Hand::Right
    }
}

pub(super) fn layered(block: &Block, level: &str) -> BlockStateId {
    block
        .from_properties(&[("level", level)])
        .to_state_id(block)
}

pub(super) fn named_shulker() -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::RED_SHULKER_BOX);
    stack.set_data_component(CustomNameImpl {
        name: TextComponent::text("Supplies"),
    });
    stack.set_data_component(ContainerImpl {
        items: vec![(4, ItemStack::new(7, &Item::DIAMOND))],
    });
    stack
}

pub(super) fn patterned_banner() -> ItemStack {
    let mut stack = ItemStack::new(2, &Item::WHITE_BANNER);
    stack.set_data_component(BannerPatternsImpl {
        layers: vec![
            BannerPatternLayer {
                pattern: "minecraft:stripe_top".into(),
                color: DyeColor::Blue,
            },
            BannerPatternLayer {
                pattern: "minecraft:border".into(),
                color: DyeColor::Red,
            },
        ],
    });
    stack
}

pub(super) fn dyed_armor() -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::LEATHER_CHESTPLATE);
    stack.set_data_component(DyedColorImpl { rgb: 0x2468ac });
    stack
}
