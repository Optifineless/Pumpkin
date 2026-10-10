use super::*;
use crate::{
    block::entities::{chest::ChestBlockEntity, trapped_chest::TrappedChestBlockEntity},
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    block_properties::{ChestLikeProperties, ChestType, HorizontalFacing},
};
use pumpkin_world::world::BlockFlags;

fn chest_pair(
    world: &Arc<World>,
    right: BlockPos,
) -> (Arc<ChestBlockEntity>, Arc<ChestBlockEntity>) {
    publish(world, proto(&Biome::PLAINS, &Block::STONE));
    let left = right.offset(pumpkin_util::math::vector3::Vector3::new(-1, 0, 0));
    for (position, half) in [(right, ChestType::Right), (left, ChestType::Left)] {
        let mut props = ChestLikeProperties::from_state_id(Block::CHEST.default_state.id);
        props.facing = HorizontalFacing::North;
        props.r#type = half;
        world.set_block_state(
            &position,
            props.to_state_id(&Block::CHEST),
            BlockFlags::FORCE_STATE,
        );
    }
    let first = Arc::new(ChestBlockEntity::new(right));
    let second = Arc::new(ChestBlockEntity::new(left));
    world.add_block_entity(first.clone());
    world.add_block_entity(second.clone());
    (first, second)
}

#[tokio::test]
async fn hopper_pulls_from_connected_chest_half() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 8), FacingHopper::Down);
    let (first, second) = chest_pair(&fixture.world, hopper.position.up());
    second.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND));
    assert!(hopper.suck_in_items(&fixture.world));
    assert_eq!(hopper.get_stack(0).item_count, 1);
    assert!(first.is_empty());
    assert_eq!(second.get_stack(0).item_count, 1);
    fixture.finish().await;
}

#[tokio::test]
async fn hopper_pushes_past_full_first_chest_half() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 7), FacingHopper::South);
    let (first, second) = chest_pair(&fixture.world, BlockPos::new(8, 64, 8));
    for slot in 0..first.size() {
        first.set_stack(slot, ItemStack::new(64, &pumpkin_data::item::Item::STONE));
    }
    hopper.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND));
    assert!(hopper.eject_items(&fixture.world, FacingHopper::South));
    assert_eq!(hopper.get_stack(0).item_count, 1);
    assert_eq!(second.get_stack(0).item_count, 1);
    assert_eq!(first.get_stack(0).item_count, 64);
    fixture.finish().await;
}

#[tokio::test]
async fn malformed_chest_partner_is_not_combined() {
    let fixture = Fixture::new();
    let right = BlockPos::new(8, 64, 8);
    let (first, second) = chest_pair(&fixture.world, right);
    let partner = second.position;
    assert_eq!(
        get_container_at(&fixture.world, &partner).unwrap().size(),
        54
    );
    for (block, half, facing) in [
        (Block::CHEST, ChestType::Right, HorizontalFacing::North),
        (Block::CHEST, ChestType::Single, HorizontalFacing::North),
        (Block::CHEST, ChestType::Left, HorizontalFacing::South),
        (
            Block::TRAPPED_CHEST,
            ChestType::Left,
            HorizontalFacing::North,
        ),
    ] {
        let mut props = ChestLikeProperties::from_state_id(block.default_state.id);
        props.r#type = half;
        props.facing = facing;
        fixture
            .world
            .set_block_state(&partner, props.to_state_id(&block), BlockFlags::FORCE_STATE);
        if block == Block::TRAPPED_CHEST {
            fixture
                .world
                .add_block_entity(Arc::new(TrappedChestBlockEntity::new(partner)));
        }
        assert_eq!(
            get_container_at(&fixture.world, &right).unwrap().size(),
            first.size()
        );
    }
    let mut props = ChestLikeProperties::from_state_id(Block::CHEST.default_state.id);
    props.facing = HorizontalFacing::North;
    props.r#type = ChestType::Left;
    fixture.world.set_block_state(
        &partner,
        props.to_state_id(&Block::CHEST),
        BlockFlags::FORCE_STATE,
    );
    fixture.world.add_block_entity(Arc::new(
        crate::block::entities::barrel::BarrelBlockEntity::new(partner),
    ));
    assert_eq!(
        get_container_at(&fixture.world, &right).unwrap().size(),
        first.size()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn failed_double_chest_transfer_preserves_source() {
    let fixture = Fixture::new();
    let hopper = HopperBlockEntity::new(BlockPos::new(8, 64, 7), FacingHopper::South);
    let chests: [_; 2] = chest_pair(&fixture.world, BlockPos::new(8, 64, 8)).into();
    for chest in chests {
        for slot in 0..chest.size() {
            chest.set_stack(slot, ItemStack::new(64, &pumpkin_data::item::Item::STONE));
        }
    }
    let source = ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND);
    hopper.set_stack(0, source.clone());
    assert!(!hopper.eject_items(&fixture.world, FacingHopper::South));
    assert!(hopper.get_stack(0).are_equal(&source));
    assert!(fixture.world.entities.load().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn opposite_half_hoppers_concurrently_drain_without_duplication() {
    let fixture = Fixture::new();
    let right = BlockPos::new(8, 65, 8);
    let (first, second) = chest_pair(&fixture.world, right);
    let hoppers = [
        HopperBlockEntity::new(first.position.down(), FacingHopper::Down),
        HopperBlockEntity::new(second.position.down(), FacingHopper::Down),
    ];
    let barrier = std::sync::Barrier::new(2);
    let conserved = std::sync::atomic::AtomicBool::new(true);
    first.set_stack(0, ItemStack::new(64, &pumpkin_data::item::Item::DIAMOND));
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let hoppers = &hoppers;
                let barrier = &barrier;
                let conserved = &conserved;
                let world = &fixture.world;
                let first = &first;
                let second = &second;
                scope.spawn(move || {
                    for _ in 0..256 {
                        barrier.wait();
                        for _ in 0..32 {
                            hoppers[index].suck_in_items(world);
                        }
                        barrier.wait();
                        if index == 0 {
                            let total: u32 = [
                                &**first as &dyn Inventory,
                                &**second,
                                &hoppers[0],
                                &hoppers[1],
                            ]
                            .into_iter()
                            .map(|inventory| {
                                (0..inventory.size())
                                    .map(|slot| u32::from(inventory.get_stack(slot).item_count))
                                    .sum::<u32>()
                            })
                            .sum();
                            if total != 64 {
                                conserved.store(false, Ordering::Relaxed);
                            }
                            first.clear();
                            second.clear();
                            hoppers[0].clear();
                            hoppers[1].clear();
                            first.set_stack(
                                0,
                                ItemStack::new(64, &pumpkin_data::item::Item::DIAMOND),
                            );
                        }
                        barrier.wait();
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
    assert!(
        conserved.load(Ordering::Relaxed),
        "both halves must conserve their shared items"
    );
    fixture.finish().await;
}

// Pause inside the slot mutation: the old fallback exposes its read snapshot to a second writer.
fn interleave_slot_update<const N: usize>(
    inventory: &dyn Inventory,
    storage: &std::sync::RwLock<[ItemStack; N]>,
    extract: bool,
) {
    use pumpkin_data::item::Item;
    use std::{
        sync::{Barrier, mpsc},
        time::Duration,
    };
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let (started, started_rx) = mpsc::channel();
    let (done, done_rx) = mpsc::channel();
    if extract {
        inventory.set_stack(0, ItemStack::new(1, &Item::DIAMOND));
    }
    let (extracted, storage_locked) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            with_inventory_slot(inventory, 0, |stack| {
                entered.wait();
                release.wait();
                let mut one = ItemStack::new(1, &Item::DIAMOND);
                let max = one.get_max_stack_size();
                assert!(HopperBlockEntity::merge_into_slot(stack, &mut one, max));
            });
        });
        entered.wait();
        // The barrier holds the mutating callback open; this cannot depend on worker scheduling.
        let storage_locked = matches!(
            storage.try_write(),
            Err(std::sync::TryLockError::WouldBlock)
        );
        let second = scope.spawn(|| {
            started.send(()).unwrap();
            let removed = if extract {
                HopperBlockEntity::take_from(inventory, 0)
                    .unwrap()
                    .one_item
                    .item_count
            } else {
                let leftover =
                    HopperBlockEntity::add_item(None, inventory, ItemStack::new(1, &Item::DIAMOND));
                assert!(leftover.is_empty());
                0
            };
            done.send(removed).unwrap();
            removed
        });
        started_rx.recv().unwrap();
        // Fixed code blocks on the paused storage lock; defective code finishes from a stale clone.
        let _ = done_rx.recv_timeout(Duration::from_secs(1));
        release.wait();
        first.join().unwrap();
        (second.join().unwrap(), storage_locked)
    });
    assert!(
        storage_locked,
        "the callback must own the real container slot lock"
    );
    assert_eq!(
        u32::from(inventory.get_stack(0).item_count) + u32::from(extracted),
        2
    );
}

#[test]
fn double_chest_concurrent_insertions_conserve_items() {
    let first = Arc::new(ChestBlockEntity::new(BlockPos::new(8, 64, 8)));
    let second = Arc::new(ChestBlockEntity::new(BlockPos::new(7, 64, 8)));
    let combined = pumpkin_inventory::double::DoubleInventory::new(first.clone(), second);
    interleave_slot_update(combined.as_ref(), &first.items, false);
}

#[test]
fn barrel_insert_vs_extract_conserves_items() {
    let barrel = crate::block::entities::barrel::BarrelBlockEntity::new(BlockPos::new(8, 64, 8));
    interleave_slot_update(&barrel, &barrel.items, true);
}

#[test]
fn crafter_empty_extraction_preserves_disabled_slot() {
    use crate::block::entities::crafter::CrafterBlockEntity;
    let crafter = CrafterBlockEntity::new(BlockPos::new(8, 64, 8));
    crafter.set_slot_state(0, false);
    assert!(HopperBlockEntity::take_from(&crafter, 0).is_none());
    assert!(crafter.is_slot_disabled(0));
    // A rollback that actually restores an item retains CrafterBlockEntity.setItem's re-enable.
    with_inventory_slot(&crafter, 0, |stack| {
        *stack = ItemStack::new(1, &pumpkin_data::item::Item::DIAMOND);
    });
    assert!(!crafter.is_slot_disabled(0));
    assert_eq!(crafter.get_stack(0).item_count, 1);
}

#[test]
fn furnace_locked_slot_update_preserves_input_timer_reset() {
    use crate::block::entities::{
        furnace::FurnaceBlockEntity, furnace_like_block_entity::CookingBlockEntityBase,
    };
    let furnace = FurnaceBlockEntity::new(BlockPos::new(8, 64, 8));
    furnace.set_stack(0, ItemStack::new(1, &pumpkin_data::item::Item::STONE));
    furnace.set_cooking_time_spent(37);
    with_inventory_slot(&furnace, 0, |stack| stack.increment(1));
    assert_eq!(furnace.get_cooking_time_spent(), 37);
    with_inventory_slot(&furnace, 0, |stack| stack.decrement(2));
    assert_eq!(furnace.get_cooking_time_spent(), 0);
}

#[test]
fn chest_rollback_preserves_a_concurrent_replacement() {
    let chest = ChestBlockEntity::new(BlockPos::new(8, 64, 8));
    chest.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::DIAMOND));
    let extraction = HopperBlockEntity::take_from(&chest, 0).unwrap();
    chest.set_stack(0, ItemStack::new(64, &pumpkin_data::item::Item::STONE));
    let leftover = HopperBlockEntity::restore_to(&chest, 0, extraction).unwrap();
    assert_eq!(leftover.item_count, 1);
    assert_eq!(leftover.item, &pumpkin_data::item::Item::DIAMOND);
    assert_eq!(chest.get_stack(0).item_count, 64);
    assert_eq!(chest.get_stack(0).item, &pumpkin_data::item::Item::STONE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_hopper_empty_barrel_polling_never_marks_dirty_or_updates_comparator() {
    use crate::{
        block::entities::barrel::BarrelBlockEntity, entity::death_test_world::DeathTestWorld,
    };
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &pos,
        Block::HOPPER.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    world.set_block_state(
        &pos.up(),
        Block::BARREL.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let hopper = Arc::new(HopperBlockEntity::new(pos, FacingHopper::Down));
    let barrel = Arc::new(BarrelBlockEntity::new(pos.up()));
    world.add_block_entity(hopper.clone());
    world.add_block_entity(barrel.clone());
    let comparator = pos
        .up()
        .offset(pumpkin_util::math::vector3::Vector3::new(1, 0, 0));
    world.set_block_state(
        &comparator,
        Block::COMPARATOR.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    hopper.clear_dirty();
    hopper.clear_comparator_dirty();
    barrel.clear_dirty();
    barrel.clear_comparator_dirty();
    for _ in 0..100 {
        // Also probe the locked path when another hopper emptied a previously read slot.
        for slot in 0..barrel.size() {
            assert!(HopperBlockEntity::take_from(barrel.as_ref(), slot).is_none());
        }
        hopper.tick(&world);
        assert!(!barrel.is_dirty(), "empty extraction dirtied the barrel");
        assert!(!barrel.is_comparator_dirty());
        assert!(!hopper.is_dirty());
        assert!(!hopper.is_comparator_dirty());
    }
    fixture.server.shutdown().await;
}

#[test]
fn hf3_container_slot_noops_and_invalid_slots_do_not_dirty() {
    use crate::block::entities::{
        barrel::BarrelBlockEntity, brewing_stand::BrewingStandBlockEntity,
        crafter::CrafterBlockEntity, dispenser::DispenserBlockEntity, dropper::DropperBlockEntity,
        furnace::FurnaceBlockEntity, shulker_box::ShulkerBoxBlockEntity,
    };
    let pos = BlockPos::new(8, 64, 8);
    let cases: Vec<Arc<dyn BlockEntity>> = vec![
        Arc::new(BarrelBlockEntity::new(pos)),
        Arc::new(ChestBlockEntity::new(pos)),
        Arc::new(TrappedChestBlockEntity::new(pos)),
        Arc::new(HopperBlockEntity::new(pos, FacingHopper::Down)),
        Arc::new(CrafterBlockEntity::new(pos)),
        Arc::new(FurnaceBlockEntity::new(pos)),
        Arc::new(BrewingStandBlockEntity::new(pos)),
        Arc::new(DispenserBlockEntity::new(pos)),
        Arc::new(DropperBlockEntity::new(pos)),
        Arc::new(ShulkerBoxBlockEntity::new(pos)),
    ];
    for entity in cases {
        let inventory = entity.clone().get_inventory().unwrap();
        entity.clear_dirty();
        entity.clear_comparator_dirty();
        inventory.update_slot(inventory.size(), &mut |_| panic!("invalid slot callback"));
        inventory.update_slot(usize::MAX, &mut |_| panic!("invalid slot callback"));
        inventory.update_slot(0, &mut |_| {});
        assert!(!entity.is_dirty(), "{} noop", entity.resource_location());
        assert!(
            !entity.is_comparator_dirty(),
            "{} noop",
            entity.resource_location()
        );
        inventory.update_slot(0, &mut |stack| {
            *stack = ItemStack::new(1, &pumpkin_data::item::Item::DIAMOND);
        });
        assert!(entity.is_dirty(), "{} mutation", entity.resource_location());
    }
}

#[test]
fn hf3_furnace_and_chest_unchanged_slot_updates_do_not_dirty() {
    let pos = BlockPos::new(8, 64, 8);
    let furnace = Arc::new(crate::block::entities::furnace::FurnaceBlockEntity::new(
        pos,
    ));
    // Arrange a clean, populated furnace without invoking setItem's dirty mark.
    furnace.items.write().unwrap()[0] = ItemStack::new(2, &pumpkin_data::item::Item::STONE);
    let chest = Arc::new(ChestBlockEntity::new(pos));
    chest.set_stack(0, ItemStack::new(2, &pumpkin_data::item::Item::STONE));
    let entities: [Arc<dyn BlockEntity>; 2] = [furnace, chest];
    for entity in entities {
        let inventory = entity.clone().get_inventory().unwrap();
        entity.clear_dirty();
        entity.clear_comparator_dirty();
        inventory.update_slot(0, &mut |stack| {
            stack.decrement(1);
            stack.increment(1);
        });
        assert!(
            !entity.is_dirty(),
            "{} unchanged",
            entity.resource_location()
        );
        assert!(!entity.is_comparator_dirty());
    }
}
