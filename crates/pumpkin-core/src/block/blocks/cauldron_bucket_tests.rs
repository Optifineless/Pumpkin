//! Real Java use-on regressions; the matrix follows CauldronInteractions.addDefaultInteractions.
use std::sync::{Arc, atomic::Ordering};

use crate::{
    block::registry::BlockActionResult,
    item::items::glass_bottle::water_bottle,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        block::cauldron_level_change::{CauldronChangeReason, CauldronLevelChangeEvent},
    },
    server::Server,
};
use pumpkin_data::{
    Block, BlockStateId,
    item::Item,
    item_stack::ItemStack,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand, math::vector2::Vector2};

#[path = "cauldron_bucket_test_support.rs"]
mod support;
use support::{Fixture, POS, layered, other_hand};

#[path = "cauldron_bucket_effect_tests.rs"]
mod effects;

fn states() -> [(BlockStateId, i32); 8] {
    [
        (layered(&Block::WATER_CAULDRON, "1"), 1),
        (layered(&Block::WATER_CAULDRON, "2"), 2),
        (Block::CAULDRON.default_state.id, 0),
        (layered(&Block::WATER_CAULDRON, "3"), 3),
        (Block::LAVA_CAULDRON.default_state.id, 3),
        (layered(&Block::POWDER_SNOW_CAULDRON, "1"), 1),
        (layered(&Block::POWDER_SNOW_CAULDRON, "2"), 2),
        (layered(&Block::POWDER_SNOW_CAULDRON, "3"), 3),
    ]
}

fn fillings() -> [(&'static Item, BlockStateId); 3] {
    [
        (&Item::WATER_BUCKET, layered(&Block::WATER_CAULDRON, "3")),
        (&Item::LAVA_BUCKET, Block::LAVA_CAULDRON.default_state.id),
        (
            &Item::POWDER_SNOW_BUCKET,
            layered(&Block::POWDER_SNOW_CAULDRON, "3"),
        ),
    ]
}

fn assert_fill(f: &mut Fixture, hand: Hand, item: &Item, target: BlockStateId) {
    assert_eq!(
        f.world.get_block_state_id(&POS),
        target,
        "bucket {}",
        item.registry_key
    );
    assert_eq!(
        f.client.player.inventory().get_stack_in_hand(hand).item,
        &Item::BUCKET
    );
    assert_eq!(
        f.client
            .player
            .inventory()
            .get_stack_in_hand(other_hand(hand))
            .item,
        &Item::DIAMOND_SWORD
    );
    let stats = f.client.player.stats.lock().unwrap();
    assert_eq!(
        stats.get(
            StatisticCategory::Custom,
            CustomStatistic::FillCauldron as i32
        ),
        1
    );
    assert_eq!(stats.get(StatisticCategory::Used, i32::from(item.id)), 1);
    drop(stats);
    assert_eq!(f.sound_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_29_filled_buckets_top_up_and_replace_all_contents_both_hands() {
    let mut f = Fixture::new();
    for hand in Hand::all() {
        for (state, level) in states() {
            for (item, target) in fillings() {
                f.reset(state, hand, ItemStack::new(1, item));
                f.use_top(hand);
                assert_fill(&mut f, hand, item, target);
                assert_eq!(
                    *f.events.changes.lock().unwrap(),
                    [(state, level, 3, CauldronChangeReason::BucketEmpty)]
                );
                assert_eq!(
                    *f.events.game_events.lock().unwrap(),
                    [pumpkin_data::game_event::GameEvent::FluidPlace.name()]
                );
            }
        }
    }
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_control_dry_empty_fills_both_hands() {
    let mut f = Fixture::new();
    for hand in Hand::all() {
        for (item, target) in fillings() {
            f.reset(
                Block::CAULDRON.default_state.id,
                hand,
                ItemStack::new(1, item),
            );
            f.use_top(hand);
            assert_fill(&mut f, hand, item, target);
        }
    }
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_30_cancel_water_lava_snow_before_all_effects() {
    let mut f = Fixture::new();
    f.events.cancel.store(true, Ordering::Relaxed);
    for mode in [GameMode::Survival, GameMode::Creative] {
        f.client.player.gamemode.store(mode);
        for hand in Hand::all() {
            let mut targets = states();
            targets.swap(0, 2);
            for (state, level) in targets {
                for (item, _) in fillings() {
                    f.reset(state, hand, ItemStack::new(2, item));
                    let inventory = f.inventory();
                    f.use_top(hand);
                    assert_eq!(
                        *f.events.changes.lock().unwrap(),
                        [(state, level, 3, CauldronChangeReason::BucketEmpty)],
                        "event for {}",
                        item.registry_key
                    );
                    f.assert_unchanged(state, &inventory);
                }
            }
        }
    }
    assert!(matches!(
        f.block_result(Hand::Left),
        BlockActionResult::Consume
    ));
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_31_submerged_lava_snow_consume_without_effects() {
    let mut f = Fixture::new();
    for hand in Hand::all() {
        for above in [
            Block::WATER.default_state.id,
            Block::WATER
                .from_properties(&[("level", "5")])
                .to_state_id(&Block::WATER),
            Block::OAK_SLAB
                .from_properties(&[("waterlogged", "true")])
                .to_state_id(&Block::OAK_SLAB),
        ] {
            let mut targets = states();
            targets.swap(0, 2);
            for (state, _) in targets {
                for item in [&Item::LAVA_BUCKET, &Item::POWDER_SNOW_BUCKET] {
                    f.reset(state, hand, ItemStack::new(2, item));
                    f.put(POS.up(), above);
                    f.clear_effects();
                    let inventory = f.inventory();
                    f.use_top(hand);
                    f.assert_unchanged(state, &inventory);
                    assert!(matches!(f.block_result(hand), BlockActionResult::Consume));
                    assert!(f.events.changes.lock().unwrap().is_empty());
                    assert_eq!(f.world.get_block_state_id(&POS.up()), above);
                }
            }
        }
    }
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_control_water_fill_underwater_and_lava_above() {
    let mut f = Fixture::new();
    for hand in Hand::all() {
        for (item, target) in fillings() {
            f.reset(
                Block::CAULDRON.default_state.id,
                hand,
                ItemStack::new(1, item),
            );
            f.put(
                POS.up(),
                if item == &Item::WATER_BUCKET {
                    Block::WATER.default_state.id
                } else {
                    Block::LAVA.default_state.id
                },
            );
            f.clear_effects();
            f.use_top(hand);
            assert_fill(&mut f, hand, item, target);
        }
    }
    f.stop().await;
}

#[derive(Clone, Copy)]
enum Edit {
    Select,
    Replace,
    Identical,
    Move,
    Target,
    Submerge,
    ResetLife,
}
struct CallbackEdit {
    hand: Hand,
    edit: Edit,
}
impl EventHandler<CauldronLevelChangeEvent> for CallbackEdit {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        e: &'a mut CauldronLevelChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = e.entity.as_ref().unwrap().get_player().unwrap();
            assert!(
                player.stats.lock().unwrap().stats.is_empty(),
                "event precedes statistics"
            );
            let inv = player.inventory();
            let slot = if self.hand == Hand::Right { 0 } else { 40 };
            let old = inv.get_stack(slot);
            match self.edit {
                Edit::Select => inv.set_selected_slot(1),
                Edit::ResetLife => player.living_entity.reset_state(),
                Edit::Replace => inv.set_stack(slot, ItemStack::new(7, &Item::DIAMOND)),
                Edit::Identical => inv.set_stack(slot, ItemStack::new(old.item_count, old.item)),
                Edit::Move => {
                    inv.set_stack(10, old);
                    inv.set_stack(slot, ItemStack::EMPTY.clone());
                }
                Edit::Target => {
                    e.world.set_block_state(
                        &POS,
                        Block::GOLD_BLOCK.default_state.id,
                        pumpkin_world::world::BlockFlags::FORCE_STATE,
                    );
                }
                Edit::Submerge => {
                    e.world.set_block_state(
                        &POS.up(),
                        Block::WATER.default_state.id,
                        pumpkin_world::world::BlockFlags::FORCE_STATE,
                    );
                }
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_callbacks_preserve_replacements_and_pin_originating_slot() {
    for hand in Hand::all() {
        for edit in [Edit::Select, Edit::Replace, Edit::Identical, Edit::Move] {
            let mut f = Fixture::new();
            f.server
                .plugin_manager
                .register::<CauldronLevelChangeEvent, _>(
                    Arc::new(CallbackEdit { hand, edit }),
                    EventPriority::High,
                    true,
                );
            for count in [1, 2] {
                for (item, target) in fillings() {
                    let state = Block::CAULDRON.default_state.id;
                    f.reset(state, hand, ItemStack::new(count, item));
                    f.client
                        .player
                        .inventory()
                        .set_stack(1, ItemStack::new(5, &Item::EMERALD));
                    let source_slot = if hand == Hand::Right { 0 } else { 40 };
                    let original = f.client.player.inventory().get_stack(source_slot);
                    f.use_top(hand);
                    let inv = f.client.player.inventory().clone();
                    let slot = if hand == Hand::Right { 0 } else { 40 };
                    if matches!(edit, Edit::Select) {
                        assert_eq!(f.world.get_block_state_id(&POS), target);
                        assert_eq!(inv.get_stack(slot).item_count, 1);
                        assert_eq!(f.count(&Item::BUCKET), 1);
                        assert_eq!(f.count(item), u32::from(count - 1));
                        assert_eq!(inv.get_stack(1).item_count, 5);
                        assert_eq!(inv.get_selected_slot(), 1);
                    } else {
                        assert_eq!(
                            f.world.get_block_state_id(&POS),
                            state,
                            "stale source must not fill"
                        );
                        assert_eq!(f.count(&Item::BUCKET), 0);
                        assert!(f.client.player.stats.lock().unwrap().stats.is_empty());
                        assert_eq!(f.sound_count(), 0);
                        assert!(f.events.game_events.lock().unwrap().is_empty());
                        match edit {
                            Edit::Replace => assert_eq!(inv.get_stack(slot).item, &Item::DIAMOND),
                            Edit::Identical => {
                                assert_eq!(inv.get_stack(slot).item_count, count);
                                assert_ne!(inv.get_stack(slot).uid, original.uid);
                            }
                            Edit::Move => {
                                assert!(inv.get_stack(slot).is_empty());
                                assert_eq!(inv.get_stack(10).item_count, count);
                                assert_eq!(inv.get_stack(10).uid, original.uid);
                            }
                            _ => panic!("expected a source replacement case"),
                        }
                    }
                }
            }
            f.stop().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_callbacks_revalidate_target_and_above_water() {
    for edit in [Edit::Target, Edit::Submerge, Edit::ResetLife] {
        let mut f = Fixture::new();
        f.server
            .plugin_manager
            .register::<CauldronLevelChangeEvent, _>(
                Arc::new(CallbackEdit {
                    hand: Hand::Left,
                    edit,
                }),
                EventPriority::High,
                true,
            );
        for item in [&Item::LAVA_BUCKET, &Item::POWDER_SNOW_BUCKET] {
            f.reset(
                Block::CAULDRON.default_state.id,
                Hand::Left,
                ItemStack::new(1, item),
            );
            let inventory = f.inventory();
            f.use_top(Hand::Left);
            f.assert_unchanged(
                if matches!(edit, Edit::Target) {
                    Block::GOLD_BLOCK.default_state.id
                } else {
                    Block::CAULDRON.default_state.id
                },
                &inventory,
            );
        }
        f.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_stacked_survival_and_creative_inventory_accounting() {
    let mut f = Fixture::new();
    for mode in [GameMode::Survival, GameMode::Creative] {
        f.client.player.gamemode.store(mode);
        for hand in Hand::all() {
            for full in [false, true] {
                for (item, target) in fillings() {
                    f.reset(
                        Block::CAULDRON.default_state.id,
                        hand,
                        ItemStack::new(2, item),
                    );
                    if full {
                        for slot in 0..36 {
                            f.client
                                .player
                                .inventory()
                                .set_stack(slot, ItemStack::new(64, &Item::STONE));
                        }
                        f.client
                            .player
                            .inventory()
                            .set_stack_in_hand(hand, ItemStack::new(2, item));
                    }
                    f.use_top(hand);
                    assert_eq!(f.world.get_block_state_id(&POS), target);
                    assert_eq!(
                        f.count(item),
                        if mode == GameMode::Creative { 2 } else { 1 }
                    );
                    assert_eq!(f.count(&Item::BUCKET), u32::from(!full));
                    let drops = f.drops();
                    assert_eq!(drops.len(), usize::from(full && mode == GameMode::Survival));
                    if let Some(drop) = drops.first() {
                        assert_eq!(drop.item, &Item::BUCKET);
                        assert_eq!(drop.item_count, 1);
                    }
                    if mode == GameMode::Creative && !full {
                        f.use_top(hand);
                        assert_eq!(
                            f.count(&Item::BUCKET),
                            1,
                            "Creative duplicate output suppressed"
                        );
                    }
                }
            }
        }
    }
    f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_control_bottles_and_washing_keep_existing_routes() {
    let mut f = Fixture::new();
    for hand in Hand::all() {
        f.reset(Block::CAULDRON.default_state.id, hand, water_bottle());
        f.use_top(hand);
        assert_eq!(
            f.world.get_block_state_id(&POS),
            layered(&Block::WATER_CAULDRON, "1")
        );
        assert_eq!(f.count(&Item::GLASS_BOTTLE), 1);
        f.reset(layered(&Block::WATER_CAULDRON, "2"), hand, water_bottle());
        f.use_top(hand);
        assert_eq!(
            f.world.get_block_state_id(&POS),
            layered(&Block::WATER_CAULDRON, "3")
        );
        f.reset(
            layered(&Block::WATER_CAULDRON, "2"),
            hand,
            ItemStack::new(1, &Item::GLASS_BOTTLE),
        );
        f.use_top(hand);
        assert_eq!(
            f.world.get_block_state_id(&POS),
            layered(&Block::WATER_CAULDRON, "1")
        );
        assert_eq!(f.count(&Item::POTION), 1);
        let mut armor = ItemStack::new(1, &Item::LEATHER_CHESTPLATE);
        armor
            .set_data_component(pumpkin_data::data_component_impl::DyedColorImpl { rgb: 0x123456 });
        f.reset(layered(&Block::WATER_CAULDRON, "2"), hand, armor);
        f.use_top(hand);
        assert_eq!(
            f.world.get_block_state_id(&POS),
            layered(&Block::WATER_CAULDRON, "1")
        );
        assert!(
            f.client
                .player
                .inventory()
                .get_stack_in_hand(hand)
                .get_data_component::<pumpkin_data::data_component_impl::DyedColorImpl>()
                .is_none()
        );
    }
    f.stop().await;
}

struct ClearOnPhysics {
    player: Arc<crate::entity::player::Player>,
    slot: usize,
    ran: std::sync::atomic::AtomicBool,
}

impl EventHandler<crate::plugin::block::block_physics::BlockPhysicsEvent> for ClearOnPhysics {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut crate::plugin::block::block_physics::BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            use crate::command::{CommandSender, context::command_source::CommandSource};
            if event.changed_pos != POS || self.ran.swap(true, Ordering::Relaxed) {
                return;
            }
            assert_eq!(
                self.player.inventory().get_stack(self.slot).item,
                &Item::BUCKET,
                "world callbacks see the paid bucket"
            );
            assert_eq!(
                self.player.stats.lock().unwrap().get(
                    StatisticCategory::Custom,
                    CustomStatistic::FillCauldron as i32
                ),
                1
            );
            let source = CommandSource::new(
                CommandSender::Console,
                self.player.world(),
                Some(self.player.clone()),
                self.player.position(),
                Vector2::default(),
                self.player.gameprofile.name.clone(),
                pumpkin_util::text::TextComponent::text("cauldron test"),
                server.clone(),
            );
            server
                .command_dispatcher
                .load()
                .handle_command(&source, "clear @s");
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g07_physics_clear_observes_result_and_cannot_resurrect_bucket() {
    for hand in Hand::all() {
        let mut f = Fixture::new();
        let handler = Arc::new(ClearOnPhysics {
            player: f.client.player.clone(),
            slot: if hand == Hand::Right { 0 } else { 40 },
            ran: true.into(),
        });
        f.server
            .plugin_manager
            .register::<crate::plugin::block::block_physics::BlockPhysicsEvent, _>(
                handler.clone(),
                EventPriority::Normal,
                true,
            );
        for (item, target) in fillings() {
            f.reset(
                Block::CAULDRON.default_state.id,
                hand,
                ItemStack::new(1, item),
            );
            handler.ran.store(false, Ordering::Relaxed);
            f.use_top(hand);
            assert!(
                handler.ran.load(Ordering::Relaxed),
                "real neighbor callback ran"
            );
            assert_eq!(f.world.get_block_state_id(&POS), target);
            assert_eq!(
                f.count(&Item::BUCKET),
                0,
                "clear removed the already delivered result"
            );
            assert_eq!(f.count(item), 0);
            assert!(f.drops().is_empty());
        }
        f.stop().await;
    }
}
