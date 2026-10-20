#![expect(clippy::unwrap_used, reason = "Real workstation packet regressions")]

#[path = "loom_workstation_test_support.rs"]
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    data_component_impl::{BannerPatternLayer, BannerPatternsImpl, CustomNameImpl},
    dye_color::DyeColor,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::{Inventory, screen_handler::SharedScreenHandler};
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::bedrock::{
    network_item::ContainerName, server::item_stack_request::ItemStackRequestAction as Action,
};

use crate::{
    block::{BlockBehaviour, GetScreenHandlerFactoryArgs, blocks::loom::LoomBlock},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_statistic_increment::PlayerStatisticIncrementEvent,
    },
    server::Server,
};
use support::{LoomFixture, assert_same_stock, slot};

fn banner(count: u8, layers: usize) -> ItemStack {
    let mut banner = ItemStack::new(count, &Item::WHITE_BANNER);
    banner.set_data_component(
        CustomNameImpl::read_data(&NbtTag::String("Keep my name".into())).unwrap(),
    );
    banner.set_data_component(BannerPatternsImpl {
        layers: (0..layers)
            .map(|_| BannerPatternLayer {
                pattern: "minecraft:stripe_top".into(),
                color: DyeColor::Blue,
            })
            .collect(),
    });
    banner
}

fn assert_result(result: &ItemStack, original: &ItemStack, pattern: &str, count: u8) {
    assert_eq!(result.item, original.item);
    assert_eq!(
        result.item_count, count,
        "ordinary Loom request must grant its paid banners"
    );
    assert_eq!(
        result.get_data_component::<CustomNameImpl>(),
        original.get_data_component::<CustomNameImpl>()
    );
    let layers = &result
        .get_data_component::<BannerPatternsImpl>()
        .unwrap()
        .layers;
    let original_layers = &original
        .get_data_component::<BannerPatternsImpl>()
        .unwrap()
        .layers;
    assert_eq!(layers.len(), original_layers.len() + 1);
    assert_eq!(&layers[..original_layers.len()], original_layers);
    assert_eq!(layers.last().unwrap().pattern, pattern);
    assert_eq!(layers.last().unwrap().color, DyeColor::Red);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_bedrock_loom_creates_and_pays_selected_banner() {
    let original = banner(4, 1);
    let mut fixture = LoomFixture::new(original.clone(), 4, None).await;
    let actions = fixture.craft_actions("bs", 2, false);
    let (status, updates) = fixture.dispatch(-10, actions);
    let result = fixture.bedrock.player.inventory().get_stack(12);
    assert_result(&result, &original, "minecraft:stripe_bottom", 2);
    assert_eq!(status, 0);
    assert_eq!(fixture.input(0).item_count, 2);
    assert_eq!(fixture.input(1).item_count, 2);
    assert!(fixture.input(2).is_empty());
    assert_result(&fixture.preview(), &original, "minecraft:stripe_bottom", 1);
    fixture.assert_updates(&updates, false);

    let mut actions = fixture.craft_actions("bs", 1, true);
    if let Action::Take { source, .. } = &mut actions[4] {
        *source = slot(ContainerName::LoomResultPreview, 50, &fixture.preview());
    }
    // Clients may omit predicted Consume actions. Authority and response input
    // addresses must still come from the server, not those descriptors.
    actions.retain(|action| !matches!(action, Action::Consume { .. }));
    if let Action::CraftResultsDeprecated { result_items, .. } = &mut actions[1] {
        // A forged proposal cannot replace the paid server banner or its layers.
        result_items[0].identifier = Some("minecraft:diamond".into());
        result_items[0].count = 64;
        result_items[0].extra_data = vec![255; 8];
    }
    let updates = fixture.submit(-10, actions, 0);
    assert_result(&fixture.cursor(), &original, "minecraft:stripe_bottom", 1);
    fixture.assert_updates(&updates, true);
    assert_eq!(fixture.input(0).item_count, 1);
    assert_eq!(fixture.input(1).item_count, 1);
    fixture.close().await;

    let mut fixture =
        LoomFixture::new(original.clone(), 4, Some(&Item::FLOWER_BANNER_PATTERN)).await;
    let pattern = fixture.input(2);
    let actions = fixture.craft_actions("flo", 2, false);
    let (status, updates) = fixture.dispatch(-10, actions);
    fixture.assert_updates(&updates, false);
    assert_result(
        &fixture.bedrock.player.inventory().get_stack(12),
        &original,
        "minecraft:flower",
        2,
    );
    assert_eq!(status, 0);
    assert_same_stock(&[fixture.input(2)], &[pattern], "pattern item retained");
    assert_eq!(fixture.input(0).item_count, 2);
    assert_eq!(fixture.input(1).item_count, 2);
    fixture.close().await;
}

#[derive(Debug)]
enum Invalid {
    Pattern,
    RequiredItem,
    MissingBanner,
    MissingDye,
    Layers,
    Repetitions,
    ZeroRepetitions,
    Full,
    Incompatible,
    DuplicateOutput,
    DuplicateConsume,
    Suffix,
    WrongUid,
    StaleSession,
    ClosedMenu,
    DuplicateMetadata,
    EmptyMetadata,
    CountMetadata,
    MisplacedMetadata,
}

fn occupy_destination(fixture: &LoomFixture, mut occupied: ItemStack, pattern: &str, full: bool) {
    if full {
        let mut layers = occupied
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .clone();
        layers.layers.push(BannerPatternLayer {
            pattern: pattern.into(),
            color: DyeColor::Red,
        });
        occupied.set_data_component(layers);
        occupied.item_count = occupied.get_max_stack_size();
    }
    fixture.bedrock.player.inventory().set_stack(12, occupied);
}

fn invalidate_actions(
    fixture: &LoomFixture,
    case: &Invalid,
    pattern: &str,
    repetitions: u8,
    actions: &mut Vec<Action>,
) {
    match case {
        Invalid::DuplicateOutput => actions.push(Action::Take {
            count: 1,
            source: slot(ContainerName::LoomResultPreview, 50, &fixture.input(0)),
            destination: slot(ContainerName::Inventory, 14, ItemStack::EMPTY),
        }),
        Invalid::DuplicateConsume => actions.insert(
            2,
            Action::Consume {
                count: repetitions,
                source: slot(ContainerName::LoomInput, 9, &fixture.input(0)),
            },
        ),
        Invalid::Suffix => actions.push(Action::CraftNonImplemented),
        Invalid::WrongUid => {
            if let Action::Consume { source, .. } = &mut actions[2] {
                source.stack_id += 1;
            }
        }
        Invalid::ClosedMenu => fixture.bedrock.player.close_handled_screen(),
        Invalid::DuplicateMetadata => actions.insert(
            2,
            fixture.craft_actions(pattern, repetitions, false).remove(1),
        ),
        Invalid::EmptyMetadata => {
            if let Action::CraftResultsDeprecated { result_items, .. } = &mut actions[1] {
                result_items.clear();
            }
        }
        Invalid::CountMetadata => {
            if let Action::CraftResultsDeprecated { times_crafted, .. } = &mut actions[1] {
                *times_crafted += 1;
            }
        }
        Invalid::MisplacedMetadata => actions.swap(1, 2),
        _ => {}
    }
    if matches!(case, Invalid::EmptyMetadata) {
        *fixture
            .screen
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap() = ItemStack::new(2, &Item::STICK);
    }
}

async fn rejected_sequence(case: Invalid) {
    let original = banner(
        2,
        if matches!(case, Invalid::Layers) {
            6
        } else {
            1
        },
    );
    let mut fixture = LoomFixture::new(original.clone(), 2, None).await;
    if matches!(case, Invalid::MissingBanner | Invalid::MissingDye) {
        let (index, name) = if matches!(case, Invalid::MissingBanner) {
            (0, ContainerName::LoomInput)
        } else {
            (1, ContainerName::LoomDye)
        };
        fixture.submit(
            -2,
            vec![Action::Take {
                count: 2,
                source: slot(name, if index == 0 { 9 } else { 10 }, &fixture.input(index)),
                destination: slot(ContainerName::Inventory, 13, ItemStack::EMPTY),
            }],
            0,
        );
        assert!(
            fixture.input(index).is_empty(),
            "ordinary input removal control"
        );
    }
    let pattern = match case {
        Invalid::Pattern => "minecraft:unknown_pattern",
        Invalid::RequiredItem => "flo",
        _ => "bs",
    };
    let repetitions = match case {
        Invalid::Repetitions => 3,
        Invalid::ZeroRepetitions => 0,
        _ => 1,
    };
    if matches!(case, Invalid::Full | Invalid::Incompatible) {
        occupy_destination(
            &fixture,
            original,
            "minecraft:stripe_bottom",
            matches!(case, Invalid::Full),
        );
    }
    let mut actions = fixture.craft_actions(pattern, repetitions, false);
    let replacement = matches!(case, Invalid::StaleSession).then(|| {
        TestPlayer::with_profile(
            &fixture.world.world(),
            fixture.bedrock.player.gameprofile.clone(),
        )
    });
    invalidate_actions(&fixture, &case, pattern, repetitions, &mut actions);
    let before = fixture.snapshot();
    let selection = fixture.selection();
    let (status, _) = fixture.dispatch(-10, actions);
    assert_same_stock(&fixture.snapshot(), &before, &case);
    assert_eq!(
        fixture.selection(),
        selection,
        "rejected request changed the selected pattern"
    );
    assert_eq!(
        fixture.bedrock.player.get_stat(
            pumpkin_data::statistic::StatisticCategory::Crafted,
            Item::WHITE_BANNER.id as i32
        ),
        0
    );
    assert_eq!(status, 1);
    fixture.assert_correction();
    if let Some(replacement) = replacement {
        replacement.client().close();
    }
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_loom_failed_sequences_conserve_all_stock() {
    for case in [
        Invalid::ClosedMenu,
        Invalid::StaleSession,
        Invalid::Pattern,
        Invalid::RequiredItem,
        Invalid::MissingBanner,
        Invalid::MissingDye,
        Invalid::Layers,
        Invalid::Repetitions,
        Invalid::ZeroRepetitions,
        Invalid::Full,
        Invalid::Incompatible,
        Invalid::DuplicateOutput,
        Invalid::DuplicateConsume,
        Invalid::Suffix,
        Invalid::WrongUid,
        Invalid::DuplicateMetadata,
        Invalid::EmptyMetadata,
        Invalid::CountMetadata,
        Invalid::MisplacedMetadata,
    ] {
        rejected_sequence(case).await;
    }
    // A valid request in the same regression prevents blanket rejection from passing.
    let mut fixture = LoomFixture::new(banner(2, 1), 2, None).await;
    let actions = fixture.craft_actions("bs", 1, false);
    fixture.submit(-10, actions, 0);
    assert_eq!(
        fixture.bedrock.player.inventory().get_stack(12).item_count,
        1
    );
    assert_eq!(fixture.input(0).item_count, 1);
    assert_eq!(fixture.input(1).item_count, 1);
    fixture.close().await;
}

struct ReplaceAfterTake {
    old: SharedScreenHandler,
    calls: Arc<AtomicUsize>,
    cancel_statistic: bool,
}

impl EventHandler<PlayerStatisticIncrementEvent> for ReplaceAfterTake {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerStatisticIncrementEvent,
    ) -> BoxFuture<'a, ()> {
        if !event.statistic_id.starts_with("Crafted:") {
            return Box::pin(async {});
        }
        assert!(
            self.old.try_lock().is_ok(),
            "loom take callback retained menu ownership"
        );
        assert!(
            event.player.current_screen_handler.try_lock().is_ok(),
            "callback retained session ownership"
        );
        self.calls.fetch_add(1, Relaxed);
        if self.cancel_statistic {
            event.cancelled = true;
            return Box::pin(async {});
        }
        event.player.close_handled_screen();
        let world = event.player.world();
        let factory = LoomBlock
            .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
                server,
                world: &world,
                block: &pumpkin_data::Block::LOOM,
                position: &support::POSITION,
                player: &event.player,
            })
            .unwrap();
        event
            .player
            .open_handled_screen(factory.as_ref(), Some(support::POSITION));
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_loom_take_callback_replaces_unlocked_menu() {
    let original = banner(2, 1);
    let mut fixture = LoomFixture::new(original.clone(), 2, None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(ReplaceAfterTake {
                old: fixture.screen.clone(),
                calls: calls.clone(),
                cancel_statistic: false,
            }),
            EventPriority::Normal,
            true,
        );
    let actions = fixture.craft_actions("bs", 1, false);
    fixture.submit(-10, actions, 0);
    assert_eq!(calls.load(Relaxed), 1);
    fixture.assert_no_stale_window_after_close();
    assert_result(
        &fixture.bedrock.player.inventory().get_stack(12),
        &original,
        "minecraft:stripe_bottom",
        1,
    );
    let current = fixture
        .bedrock
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    assert!(!Arc::ptr_eq(&current, &fixture.screen));
    let handler = current.lock().unwrap();
    assert_eq!(
        handler.window_type(),
        Some(pumpkin_data::screen::WindowType::Loom)
    );
    assert!(
        handler.get_behaviour().slots[..4]
            .iter()
            .all(|slot| !slot.has_stack())
    );
    drop(handler);
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_loom_cancelled_statistic_and_disconnect_preserve_payment() {
    let original = banner(2, 1);
    let mut fixture = LoomFixture::new(original.clone(), 2, None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<PlayerStatisticIncrementEvent, _>(
            Arc::new(ReplaceAfterTake {
                old: fixture.screen.clone(),
                calls: calls.clone(),
                cancel_statistic: true,
            }),
            EventPriority::Normal,
            true,
        );
    let actions = fixture.craft_actions("bs", 1, false);
    let updates = fixture.submit(-10, actions, 0);
    fixture.assert_updates(&updates, false);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(
        fixture.bedrock.player.get_stat(
            pumpkin_data::statistic::StatisticCategory::Crafted,
            Item::WHITE_BANNER.id as i32
        ),
        0
    );
    assert_result(
        &fixture.bedrock.player.inventory().get_stack(12),
        &original,
        "minecraft:stripe_bottom",
        1,
    );
    assert_eq!(fixture.input(0).item_count, 1);
    assert_eq!(fixture.input(1).item_count, 1);

    fixture.bedrock.close().await;
    let actions = fixture.craft_actions("bs", 1, false);
    let before = fixture.snapshot();
    fixture.send(-10, actions);
    assert_same_stock(&fixture.snapshot(), &before, "disconnected client");
    assert_eq!(
        calls.load(Relaxed),
        1,
        "disconnected request fired a statistic callback"
    );
    fixture.close().await;
}
