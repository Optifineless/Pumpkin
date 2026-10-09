use super::*;
use crate::entity::death_test_world::DeathTestWorld;
use crate::net::java::combat_test_support::TestPlayer;
use crate::world::spawn_test_support::{proto, publish};
use pumpkin_data::{Block, biome::Biome};
use std::sync::Barrier;

async fn concurrent_swaps(insert: bool) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    let players = [TestPlayer::new(&world), TestPlayer::new(&world)];
    if !insert {
        stand.set_item_slot(
            &EquipmentSlot::HEAD,
            &ItemStack::new(1, &Item::DIAMOND_HELMET),
        );
    }
    let barrier = Barrier::new(2);
    let conserved = AtomicBool::new(true);
    std::thread::scope(|scope| {
        let workers: Vec<_> = players
            .iter()
            .enumerate()
            .map(|(index, player)| {
                let stand = &stand;
                let barrier = &barrier;
                let players = &players;
                let conserved = &conserved;
                scope.spawn(move || {
                    for _ in 0..1024 {
                        barrier.wait();
                        let mut hand = if insert {
                            ItemStack::new(1, &Item::DIAMOND_HELMET)
                        } else {
                            ItemStack::EMPTY.clone()
                        };
                        stand.swap_item(&player.player, &EquipmentSlot::HEAD, &mut hand);
                        barrier.wait();
                        // Store each result separately; the main worker checks the whole exchange.
                        player
                            .player
                            .inventory()
                            .set_stack_in_hand(pumpkin_util::Hand::Right, hand);
                        barrier.wait();
                        if index == 0 {
                            let held: u32 = players
                                .iter()
                                .map(|p| u32::from(p.player.inventory().held_item().item_count))
                                .sum();
                            let equipped =
                                u32::from(stand.item_in_slot(&EquipmentSlot::HEAD).item_count);
                            if held + equipped != if insert { 2 } else { 1 } {
                                conserved.store(false, Ordering::Relaxed);
                            }
                            stand.set_item_slot(
                                &EquipmentSlot::HEAD,
                                &if insert {
                                    ItemStack::EMPTY.clone()
                                } else {
                                    ItemStack::new(1, &Item::DIAMOND_HELMET)
                                },
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
        "concurrent exchange must conserve equipment"
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_empty_hand_removals_conserve_one_helmet() {
    concurrent_swaps(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_insertions_conserve_both_helmets() {
    concurrent_swaps(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destruction_closes_equipment_before_late_insertion() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    let player = TestPlayer::new(&world);
    stand.drop_equipment();
    let mut hand = ItemStack::new(1, &Item::DIAMOND_HELMET);
    assert!(!stand.swap_item(&player.player, &EquipmentSlot::HEAD, &mut hand));
    assert_eq!(hand.item_count, 1);
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).is_empty());
    fixture.server.shutdown().await;
}
