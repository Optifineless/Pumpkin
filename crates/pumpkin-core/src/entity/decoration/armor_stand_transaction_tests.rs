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
                        player
                            .player
                            .inventory()
                            .set_stack_in_hand(pumpkin_util::Hand::Right, hand.clone());
                        stand.swap_item(&player.player, &EquipmentSlot::HEAD, &mut hand, 0);
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
    player
        .player
        .inventory()
        .set_stack_in_hand(pumpkin_util::Hand::Right, hand.clone());
    assert!(!stand.swap_item(&player.player, &EquipmentSlot::HEAD, &mut hand, 0));
    assert_eq!(hand.item_count, 1);
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).is_empty());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_player_two_stands_commit_only_one_helmet() {
    use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
    use pumpkin_inventory::player::player_inventory::PlayerInventory;
    use pumpkin_util::Hand;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player
        .player
        .inventory()
        .set_stack_in_hand(Hand::Left, helmet.clone());
    let stands: [_; 2] = std::array::from_fn(|_| {
        ArmorStandEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 8.5),
            &pumpkin_data::entity::EntityType::ARMOR_STAND,
        ))
    });
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        let workers: Vec<_> = stands
            .iter()
            .map(|stand| {
                let player = &player.player;
                let barrier = &barrier;
                scope.spawn(move || {
                    let before = player.inventory().get_stack_in_hand(Hand::Left);
                    let mut hand = before.clone();
                    barrier.wait(); // Both interactions hold the same one-item hand snapshot.
                    stand.interact_with_hand(player, &mut hand, Hand::Left);
                    barrier.wait(); // Both stand mutations precede the old deferred hand writes.
                    write_back_hand_item(
                        player,
                        Hand::Left,
                        PlayerInventory::OFF_HAND_SLOT,
                        &before,
                        &hand,
                        HandMutation::EquipmentTransfer,
                    );
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let total: u32 = stands
        .iter()
        .map(|s| u32::from(s.item_in_slot(&EquipmentSlot::HEAD).item_count))
        .sum();
    assert_eq!(total, 1);
    assert!(player.player.inventory().off_hand_item().is_empty());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hf3_entity_hook_commits_admitted_hand_slot_after_hotbar_change() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let stand = ArmorStandEntity::new(Entity::new(
        world,
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    let mut hand = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player.player.inventory().set_stack(0, hand.clone());
    player.player.inventory().set_selected_slot(1);
    let entity: &dyn EntityBase = &stand;
    assert_eq!(
        entity.interact_from_hand_slot(&player.player, &mut hand, None, 0),
        Some(true)
    );
    assert!(player.player.inventory().get_stack(0).is_empty());
    assert!(player.player.inventory().get_stack(1).is_empty());
    assert_eq!(stand.item_in_slot(&EquipmentSlot::HEAD).item_count, 1);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_hand_cannot_mutate_stand_or_emit_equipment_events() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    let mut snapshot = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player
        .player
        .inventory()
        .set_stack_in_hand(pumpkin_util::Hand::Right, snapshot.clone());
    player
        .player
        .inventory()
        .set_stack_in_hand(pumpkin_util::Hand::Right, ItemStack::new(1, &Item::STONE));
    assert!(!stand.interact(&player.player, &mut snapshot));
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).is_empty());
    assert_eq!(player.player.inventory().held_item().item, &Item::STONE);
    fixture.server.shutdown().await;
}

struct CommittedHand(Arc<Player>);
impl crate::plugin::EventHandler<crate::plugin::api::events::world::generic_game::GenericGameEvent>
    for CommittedHand
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<crate::server::Server>,
        event: &'a mut crate::plugin::api::events::world::generic_game::GenericGameEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        if event.event_key == GameEvent::Equip.name() {
            assert!(
                self.0.inventory().off_hand_item().is_empty(),
                "equipment callback preceded hand commit"
            );
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn armor_stand_equipment_event_observes_committed_hand() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    stand.living_entity.combat_ticks.store(1, Ordering::Relaxed);
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::api::events::world::generic_game::GenericGameEvent, _>(
            Arc::new(CommittedHand(player.player.clone())),
            crate::plugin::EventPriority::Normal,
            true,
        );
    let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player
        .player
        .inventory()
        .set_stack_in_hand(pumpkin_util::Hand::Left, helmet.clone());
    assert!(stand.interact_with_hand(&player.player, &mut helmet, pumpkin_util::Hand::Left));
    assert!(helmet.is_empty());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn armor_stand_exchange_retains_captured_hotbar_slot() {
    use crate::net::java::play::hand_use_result::{HandMutation, write_back_hand_item};
    use pumpkin_util::Hand;
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let player = TestPlayer::new(&world);
    let stand = ArmorStandEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &pumpkin_data::entity::EntityType::ARMOR_STAND,
    ));
    let before = ItemStack::new(1, &Item::DIAMOND_HELMET);
    player.player.inventory().set_stack(0, before.clone());
    player.player.inventory().set_stack(1, before.clone());
    player.player.inventory().set_selected_slot(1);
    let mut after = before.clone();
    assert!(stand.interact_from_hand_slot(&player.player, &mut after, None, 0));
    write_back_hand_item(
        &player.player,
        Hand::Right,
        0,
        &before,
        &after,
        HandMutation::EquipmentTransfer,
    );
    assert!(player.player.inventory().get_stack(0).is_empty());
    assert!(player.player.inventory().get_stack(1).are_equal(&before));
    assert!(stand.item_in_slot(&EquipmentSlot::HEAD).are_equal(&before));
    fixture.server.shutdown().await;
}
