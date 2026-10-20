use super::items::ground;
use super::support::*;
use crate::block::entities::dispenser::DispenserBlockEntity;
use pumpkin_data::{
    Block, block_properties::DispenserLikeProperties, item::Item, item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_powered_dispenser_admission_owns_stock() {
    run(|f| {
        let mut p = player(f);
        let hooks = Hooks::install(&f.server, &p.player);
        for item in [
            &Item::ARROW,
            &Item::SNOWBALL,
            &Item::EGG,
            &Item::SPLASH_POTION,
            &Item::LINGERING_POTION,
            &Item::EXPERIENCE_BOTTLE,
            &Item::FIRE_CHARGE,
            &Item::WIND_CHARGE,
            &Item::FIREWORK_ROCKET,
        ] {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                for edit in 0..4 {
                    if veto == Veto::None && edit != 0 {
                        continue;
                    }
                    case(f, &mut p, &hooks, item, veto, edit);
                }
            }
        }
    })
    .await;
}

pub(super) fn case(
    f: &crate::entity::death_test_world::DeathTestWorld,
    p: &mut crate::net::java::combat_test_support::TestPlayer,
    hooks: &Arc<Hooks>,
    item: &'static Item,
    veto: Veto,
    edit: i32,
) {
    let world = f.world();
    let position = BlockPos::new(8, 64, 8);
    let power = BlockPos::new(9, 64, 8);
    let dispenser = prepare_dispenser(&world, item);
    let original = dispenser.get_stack(0);
    let callback_world = world.clone();
    let callback_dispenser = dispenser.clone();
    hooks.configure(
        veto,
        false,
        Some(Arc::new(move |_| {
            if edit == 1 {
                callback_dispenser.set_stack(0, ItemStack::new(2, &Item::DIAMOND));
            }
            if edit == 2 {
                callback_world.set_block_state(
                    &position,
                    Block::STONE.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );
            }
            if edit == 3 {
                callback_world.set_block_state(
                    &position,
                    Block::AIR.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );
                callback_world.set_block_state(
                    &position,
                    Block::DISPENSER.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
                let replacement = Arc::new(DispenserBlockEntity::new(position));
                replacement.set_stack(0, ItemStack::new(2, &Item::DIAMOND));
                callback_world.add_block_entity(replacement);
            }
        })),
    );
    hooks.state.lock().unwrap().owner = Some(dispenser.slot_transaction(0));
    let ground_before = ground(&p.player, item) + loose(&p.player, item);
    p.take_packets();
    // Power neighbor updates schedule the normal four-tick dispenser action.
    world.set_block_state(
        &power,
        Block::REDSTONE_BLOCK.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    assert!(
        world.is_block_tick_scheduled(&position, &Block::DISPENSER),
        "power did not schedule dispenser"
    );
    for _ in 0..5 {
        world.tick(&f.server);
    }
    assert_eq!(
        hooks.counts().0,
        1,
        "scheduled queue did not attempt the projectile"
    );
    let admitted = veto == Veto::None;
    let stock = dispenser.get_stack(0);
    let retained = if stock.item == item {
        usize::from(stock.item_count)
    } else {
        0
    };
    assert_eq!(
        retained + ground(&p.player, item) + loose(&p.player, item) - ground_before,
        3 - usize::from(admitted),
        "dispenser consumed rejected local stock or lost retired refund"
    );
    if edit == 0 || admitted {
        assert_eq!(stock.item_count, 3 - u8::from(admitted));
        assert!(stock.are_items_and_components_equal(&original));
    }
    if edit == 1 && !admitted {
        assert_eq!(stock.item, &Item::DIAMOND);
        assert_eq!(stock.item_count, 2);
    }
    if edit == 3 && !admitted {
        let replacement = world.get_block_entity(&position).unwrap();
        let stock = replacement.get_inventory().unwrap().get_stack(0);
        assert_eq!((stock.item, stock.item_count), (&Item::DIAMOND, 2));
    }
    let packets = p.take_packets();
    assert_eq!(
        packet_count(
            &packets,
            pumpkin_data::packet::clientbound::play::LEVEL_EVENT.0
        ),
        if admitted { 2 } else { 0 },
        "dispenser published a successful effect before admission"
    );
    let launched = hooks.state.lock().unwrap().order[0].1;
    assert_eq!(world.get_entity_by_id(launched).is_some(), admitted);
}

fn prepare_dispenser(
    world: &Arc<crate::world::World>,
    item: &'static Item,
) -> Arc<DispenserBlockEntity> {
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let position = BlockPos::new(8, 64, 8);
    let power = BlockPos::new(9, 64, 8);
    world.set_block_state(&power, Block::AIR.default_state.id, BlockFlags::NOTIFY_ALL);
    world.set_block_state(
        &position,
        Block::AIR.default_state.id,
        BlockFlags::NOTIFY_ALL,
    );
    let props = DispenserLikeProperties::default(&Block::DISPENSER);
    world.set_block_state(
        &position,
        props.to_state_id(&Block::DISPENSER),
        BlockFlags::FORCE_STATE,
    );
    let dispenser = Arc::new(DispenserBlockEntity::new(position));
    let original = stack(3, item);
    dispenser.set_stack(0, original);
    world.add_block_entity(dispenser.clone());
    dispenser
}
