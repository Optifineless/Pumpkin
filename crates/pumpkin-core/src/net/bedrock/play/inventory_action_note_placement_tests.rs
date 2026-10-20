use super::*;
use crate::plugin::{BoxFuture, EventHandler, EventPriority, block::block_place::BlockPlaceEvent};
use pumpkin_data::block_properties::NoteBlockLikeProperties;
use pumpkin_util::GameMode;
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

const NOTE: BlockPos = BlockPos::new(8, 64, 8);
const INITIAL_NOTE: u8 = 7;

#[derive(Clone, Copy, Debug)]
enum EntryPoint {
    Java,
    Bedrock,
}

const ENTRIES: [EntryPoint; 2] = [EntryPoint::Java, EntryPoint::Bedrock];

impl EntryPoint {
    fn player(self, fixture: &HandUseFixture) -> Arc<Player> {
        match self {
            Self::Java => fixture.java_observer.player.clone(),
            Self::Bedrock => fixture.owner.player.clone(),
        }
    }
}

#[derive(Default)]
struct UseEvents {
    cancel_placement: AtomicBool,
    cancel_interaction: AtomicBool,
    placements: AtomicUsize,
    interactions: AtomicUsize,
}

impl UseEvents {
    fn register(fixture: &HandUseFixture) -> Arc<Self> {
        let events = Arc::new(Self::default());
        let plugins = &fixture.world.server.plugin_manager;
        plugins.register::<BlockPlaceEvent, _>(events.clone(), EventPriority::Normal, true);
        plugins.register::<PlayerInteractEvent, _>(events.clone(), EventPriority::Normal, true);
        events
    }
}

impl EventHandler<BlockPlaceEvent> for UseEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut BlockPlaceEvent,
    ) -> BoxFuture<'a, ()> {
        assert_eq!(event.block_position, NOTE.up());
        assert_eq!(event.block_placed_against, &Block::NOTE_BLOCK);
        assert!(event.can_build);
        self.placements.fetch_add(1, SeqCst);
        event.cancelled = self.cancel_placement.load(SeqCst);
        Box::pin(async {})
    }
}

impl EventHandler<PlayerInteractEvent> for UseEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        self.interactions.fetch_add(1, SeqCst);
        event.cancelled = self.cancel_interaction.load(SeqCst);
        Box::pin(async {})
    }
}

fn prepare(
    fixture: &HandUseFixture,
    entry: EntryPoint,
    hand: Hand,
    item: Option<&'static Item>,
    creative: bool,
    blocked: bool,
) -> ItemStack {
    let player = entry.player(fixture);
    player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 10.5));
    player.inventory().set_selected_slot(4);
    player.gamemode.store(if creative {
        GameMode::Creative
    } else {
        GameMode::Survival
    });
    let stack = item.map_or_else(|| ItemStack::EMPTY.clone(), |item| ItemStack::new(2, item));
    player.inventory().set_stack_in_hand(hand, stack.clone());
    player.inventory().set_stack_in_hand(
        if hand == Hand::Right {
            Hand::Left
        } else {
            Hand::Right
        },
        ItemStack::new(3, &Item::DIAMOND),
    );
    let world = fixture.world.world();
    world.set_block_state(
        &NOTE.up(),
        if blocked {
            Block::OBSIDIAN.default_state.id
        } else {
            Block::AIR.default_state.id
        },
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let mut props = NoteBlockLikeProperties::default(&Block::NOTE_BLOCK);
    props.note = INITIAL_NOTE;
    world.set_block_state(
        &NOTE,
        props.to_state_id(&Block::NOTE_BLOCK),
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    stack
}

fn use_item(
    fixture: &HandUseFixture,
    entry: EntryPoint,
    hand: Hand,
    face: BlockDirection,
    stack: &ItemStack,
) -> TestResult {
    let player = entry.player(fixture);
    match entry {
        EntryPoint::Java => fixture.java_observer.client().handle_use_item_on(
            &player,
            &pumpkin_protocol::java::server::play::SUseItemOn {
                hand: VarInt(i32::from(hand == Hand::Left)),
                position: NOTE,
                face: VarInt(face as i32),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.world.server,
        )?,
        EntryPoint::Bedrock => {
            let (window, slot, _) = hand_address(hand);
            let mut packet = click_block_packet(
                &player,
                if hand == Hand::Right {
                    HandSlot::Mainhand
                } else {
                    HandSlot::Offhand
                },
                NOTE,
                stack,
                vec![action(
                    Some(window),
                    slot,
                    NetworkItemDescriptor::from(stack),
                    NetworkItemDescriptor::from(&stack.copy_with_count(1)),
                )],
            );
            if let TransactionData::UseItem(data) = &mut packet.transaction_data {
                data.block_face = face as u8;
            }
            fixture
                .owner
                .client()
                .handle_inventory_action(&player, packet);
        }
    }
    Ok(())
}

fn assert_result(
    fixture: &HandUseFixture,
    entry: EntryPoint,
    hand: Hand,
    note: u8,
    block: &Block,
    expected: &ItemStack,
) {
    let world = fixture.world.world();
    assert_eq!(
        NoteBlockLikeProperties::from_state_id(world.get_block_state_id(&NOTE)).note,
        note,
        "{entry:?} note value"
    );
    assert_eq!(
        world.get_block(&NOTE.up()),
        block,
        "{entry:?} top placement"
    );
    let player = entry.player(fixture);
    let inventory = player.inventory();
    let actual = inventory.get_stack_in_hand(hand);
    assert!(
        actual.are_equal(expected),
        "{entry:?} originating hand payment"
    );
    assert_eq!(
        actual.uid, expected.uid,
        "{entry:?} retained stack identity"
    );
    let other = inventory.get_stack_in_hand(if hand == Hand::Right {
        Hand::Left
    } else {
        Hand::Right
    });
    assert_eq!(other.item.id, Item::DIAMOND.id);
    assert_eq!(other.item_count, 3, "{entry:?} opposite hand payment");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_top_head_placement_bypasses_tuning() -> TestResult {
    tokio::time::timeout(Duration::from_secs(120), async {
        let fixture = HandUseFixture::new().await;
        let events = UseEvents::register(&fixture);
        fixture
            .verify_and_close(|fixture| {
                for entry in ENTRIES {
                    for hand in [Hand::Right, Hand::Left] {
                        for creative in [false, true] {
                            for (item, block) in [
                                (&Item::PLAYER_HEAD, &Block::PLAYER_HEAD),
                                (&Item::SKELETON_SKULL, &Block::SKELETON_SKULL),
                                (&Item::ZOMBIE_HEAD, &Block::ZOMBIE_HEAD),
                                (&Item::CREEPER_HEAD, &Block::CREEPER_HEAD),
                                (&Item::DRAGON_HEAD, &Block::DRAGON_HEAD),
                                (&Item::WITHER_SKELETON_SKULL, &Block::WITHER_SKELETON_SKULL),
                                (&Item::PIGLIN_HEAD, &Block::PIGLIN_HEAD),
                            ] {
                                let stack =
                                    prepare(fixture, entry, hand, Some(item), creative, false);
                                let before = events.placements.load(SeqCst);
                                use_item(fixture, entry, hand, BlockDirection::Up, &stack)?;
                                let mut expected = stack;
                                if !creative {
                                    expected.decrement(1);
                                }
                                assert_result(fixture, entry, hand, INITIAL_NOTE, block, &expected);
                                assert_eq!(events.placements.load(SeqCst), before + 1);
                            }
                        }
                    }
                }
                Ok(())
            })
            .await
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_top_head_default_tuning_controls() -> TestResult {
    tokio::time::timeout(Duration::from_secs(120), async {
        let fixture = HandUseFixture::new().await;
        let events = UseEvents::register(&fixture);
        fixture
            .verify_and_close(|fixture| {
                for entry in ENTRIES {
                    for (item, face) in [
                        (Some(&Item::SKELETON_SKULL), BlockDirection::East),
                        (Some(&Item::DIAMOND), BlockDirection::Up),
                        (None, BlockDirection::Up),
                    ] {
                        let stack = prepare(fixture, entry, Hand::Right, item, false, false);
                        use_item(fixture, entry, Hand::Right, face, &stack)?;
                        assert_result(
                            fixture,
                            entry,
                            Hand::Right,
                            INITIAL_NOTE + 1,
                            &Block::AIR,
                            &stack,
                        );
                    }
                }
                assert_eq!(events.placements.load(SeqCst), 0);
                Ok(())
            })
            .await
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g08_top_head_rejected_placement_controls() -> TestResult {
    tokio::time::timeout(Duration::from_secs(120), async {
        let fixture = HandUseFixture::new().await;
        let events = UseEvents::register(&fixture);
        fixture
            .verify_and_close(|fixture| {
                for entry in ENTRIES {
                    for hand in [Hand::Right, Hand::Left] {
                        for blocked in [true, false] {
                            let stack = prepare(
                                fixture,
                                entry,
                                hand,
                                Some(&Item::PLAYER_HEAD),
                                false,
                                blocked,
                            );
                            events.cancel_placement.store(!blocked, SeqCst);
                            let before = events.placements.load(SeqCst);
                            use_item(fixture, entry, hand, BlockDirection::Up, &stack)?;
                            assert_result(
                                fixture,
                                entry,
                                hand,
                                INITIAL_NOTE,
                                if blocked {
                                    &Block::OBSIDIAN
                                } else {
                                    &Block::AIR
                                },
                                &stack,
                            );
                            assert_eq!(
                                events.placements.load(SeqCst),
                                before + usize::from(!blocked)
                            );
                        }
                    }
                }
                events.cancel_placement.store(false, SeqCst);
                events.cancel_interaction.store(true, SeqCst);
                for hand in [Hand::Right, Hand::Left] {
                    let stack = prepare(
                        fixture,
                        EntryPoint::Java,
                        hand,
                        Some(&Item::PLAYER_HEAD),
                        false,
                        false,
                    );
                    let before = events.placements.load(SeqCst);
                    let interactions = events.interactions.load(SeqCst);
                    use_item(fixture, EntryPoint::Java, hand, BlockDirection::Up, &stack)?;
                    assert_result(
                        fixture,
                        EntryPoint::Java,
                        hand,
                        INITIAL_NOTE,
                        &Block::AIR,
                        &stack,
                    );
                    assert_eq!(events.placements.load(SeqCst), before);
                    assert_eq!(events.interactions.load(SeqCst), interactions + 1);
                }
                Ok(())
            })
            .await
    })
    .await?
}
