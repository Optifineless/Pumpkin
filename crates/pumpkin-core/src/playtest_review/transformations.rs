use super::*;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::{
    BlockDirection,
    block_properties::{
        ChestLikeProperties, ChestType, Facing, HorizontalFacing, ObserverLikeProperties,
    },
};
use pumpkin_protocol::{VarInt, java::server::play::SUseItemOn};
use pumpkin_util::Hand;

pub(super) fn use_axe(fixture: &Fixture, player: &TestPlayer, pos: BlockPos, hand: Hand) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(i32::from(hand != Hand::Right)),
                position: pos,
                face: VarInt(BlockDirection::North as i32),
                cursor_pos: Vector3::new(0.5, 0.5, 0.0),
                inside_block: false,
                sequence: VarInt(1),
                is_against_world_border: false,
            },
            &fixture.server,
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axe_respects_offhand_blocking_intent_for_both_hands_and_sneaking() {
    let fixture = Fixture::new();
    let player = TestPlayer::new(&fixture.world);
    let pos = BlockPos::new(8, 64, 9);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    for (sneaking, hand, strips) in [
        (false, Hand::Right, false),
        (true, Hand::Right, true),
        (false, Hand::Left, true),
    ] {
        fixture.world.set_block_state(
            &pos,
            Block::OAK_LOG.default_state.id,
            BlockFlags::FORCE_STATE,
        );
        player.player.get_entity().set_sneaking(sneaking);
        player.player.inventory.set_stack(
            0,
            ItemStack::new(
                1,
                if hand == Hand::Right {
                    &Item::IRON_AXE
                } else {
                    &Item::SHIELD
                },
            ),
        );
        player.player.inventory.set_stack(
            pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
            ItemStack::new(
                1,
                if hand == Hand::Right {
                    &Item::SHIELD
                } else {
                    &Item::IRON_AXE
                },
            ),
        );
        use_axe(&fixture, &player, pos, hand);
        assert_eq!(
            fixture.world.get_block(&pos),
            if strips {
                &Block::STRIPPED_OAK_LOG
            } else {
                &Block::OAK_LOG
            }
        );
        assert_eq!(
            player.player.inventory.get_stack_in_hand(hand).get_damage(),
            i32::from(strips)
        );
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stripping_a_log_schedules_an_observer_pulse() {
    let fixture = Fixture::new();
    let player = TestPlayer::new(&fixture.world);
    let log = BlockPos::new(8, 64, 9);
    let observer = BlockPos::new(9, 64, 9);
    let mut props = ObserverLikeProperties::default(&Block::OBSERVER);
    props.facing = Facing::West;
    fixture.world.set_block_state(
        &observer,
        props.to_state_id(&Block::OBSERVER),
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    fixture.world.set_block_state(
        &log,
        Block::OAK_LOG.default_state.id,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
    assert!(
        !fixture
            .world
            .is_block_tick_scheduled(&observer, &Block::OBSERVER)
    );
    use_axe(&fixture, &player, log, Hand::Right);
    assert!(
        fixture
            .world
            .is_block_tick_scheduled(&observer, &Block::OBSERVER)
    );
    // Execute the scheduled callback against the real block state, then verify the falling edge.
    for powered in [true, false] {
        fixture
            .world
            .block_registry
            .get_pumpkin_block(Block::OBSERVER.id)
            .unwrap()
            .on_scheduled_tick(crate::block::OnScheduledTickArgs {
                world: &fixture.world,
                block: &Block::OBSERVER,
                position: &observer,
            });
        assert_eq!(
            ObserverLikeProperties::from_state_id(fixture.world.get_block_state_id(&observer))
                .powered,
            powered
        );
    }
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scraping_single_and_double_copper_chests_keeps_contents_and_deferred_loot() {
    for double in [false, true] {
        let fixture = Fixture::new();
        let player = TestPlayer::new(&fixture.world);
        let pos = BlockPos::new(8, 64, 9);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 8.5));
        player.player.get_entity().set_sneaking(true);
        player
            .player
            .inventory
            .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
        let mut props = ChestLikeProperties::default(&Block::WAXED_WEATHERED_COPPER_CHEST);
        props.facing = HorizontalFacing::North;
        let positions = if double {
            vec![
                (pos, ChestType::Left),
                (BlockPos::new(9, 64, 9), ChestType::Right),
            ]
        } else {
            vec![(pos, ChestType::Single)]
        };
        let mut entities = Vec::new();
        for (position, kind) in &positions {
            props.r#type = *kind;
            fixture.world.set_block_state(
                position,
                props.to_state_id(&Block::WAXED_WEATHERED_COPPER_CHEST),
                BlockFlags::FORCE_STATE,
            );
            let entity = Arc::new(crate::block::entities::chest::ChestBlockEntity::new(
                *position,
            ));
            let mut diamonds = ItemStack::new(3, &Item::DIAMOND);
            diamonds.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
                name: pumpkin_util::text::TextComponent::text("Treasures"),
            });
            entity.set_stack(0, diamonds);
            fixture.world.add_block_entity(entity.clone());
            entities.push(entity);
        }
        // The partner's loot stays deferred until the chest is opened.
        if double {
            *entities[1].loot_table.lock().unwrap() =
                Some("minecraft:chests/simple_dungeon".to_owned());
        }
        for target in [&Block::WEATHERED_COPPER_CHEST, &Block::EXPOSED_COPPER_CHEST] {
            use_axe(&fixture, &player, pos, Hand::Right);
            for ((position, kind), entity) in positions.iter().zip(&entities) {
                assert_eq!(fixture.world.get_block(position), target);
                let after = fixture.world.get_block_entity(position).unwrap();
                assert!(Arc::ptr_eq(
                    &after,
                    &(entity.clone() as Arc<dyn crate::block::entities::BlockEntity>)
                ));
                assert_eq!(entity.get_stack(0).item_count, 3);
                assert!(
                    entity
                        .get_stack(0)
                        .get_data_component::<pumpkin_data::data_component_impl::CustomNameImpl>()
                        .is_some()
                );
                assert_eq!(
                    ChestLikeProperties::from_state_id(fixture.world.get_block_state_id(position))
                        .r#type,
                    *kind
                );
            }
            assert_eq!(fixture.drops(&Item::DIAMOND), 0);
        }
        if double {
            assert_eq!(
                entities[1].loot_table.lock().unwrap().as_deref(),
                Some("minecraft:chests/simple_dungeon")
            );
        }
        fixture.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unwaxing_preserves_pending_named_chest_nbt_and_loot_seed() {
    let fixture = Fixture::new();
    let player = TestPlayer::new(&fixture.world);
    let pos = BlockPos::new(8, 64, 9);
    fixture.world.set_block_state(
        &pos,
        Block::WAXED_COPPER_CHEST.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut nbt = NbtCompound::new();
    nbt.put_string("id", "minecraft:chest".to_owned());
    for (key, value) in [("x", 8), ("y", 64), ("z", 9)] {
        nbt.put_int(key, value);
    }
    nbt.put_string("CustomName", "Treasures".to_owned());
    nbt.put_string("LootTable", "minecraft:chests/simple_dungeon".to_owned());
    nbt.put_long("LootTableSeed", 731);
    fixture.world.add_block_entity_nbt(pos, &nbt);
    player.player.get_entity().set_sneaking(true);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    player
        .player
        .inventory
        .set_stack(0, ItemStack::new(1, &Item::IRON_AXE));
    use_axe(&fixture, &player, pos, Hand::Right);
    fixture
        .world
        .level
        .read_chunk_sync(&Vector2::new(0, 0), |chunk| {
            let pending = chunk.pending_block_entities.lock().unwrap();
            let saved = pending.get(&pos).unwrap();
            assert_eq!(saved.get_string("CustomName"), Some("Treasures"));
            assert_eq!(
                saved.get_string("LootTable"),
                Some("minecraft:chests/simple_dungeon")
            );
            assert_eq!(saved.get_long("LootTableSeed"), Some(731));
        })
        .unwrap();
    assert_eq!(fixture.world.get_block(&pos), &Block::COPPER_CHEST);
    fixture.shutdown().await;
}
