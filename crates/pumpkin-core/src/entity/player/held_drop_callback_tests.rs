use super::{
    Arc, AtomicUsize, BoxFuture, Callback, EntityBase, EventHandler, EventPriority, Fixture, Item,
    ItemStack, Ordering, Player, PlayerDropItemEvent, Route, SAttack, Server, Vector3,
    combat_values, register_callback, weapon,
};
use crate::{entity::RemovalReason, net::PlayerConfig, world::guard_effects};
use pumpkin_data::data_component_impl::{CustomNameImpl, EquipmentSlot, UnbreakableImpl};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_util::text::TextComponent;

#[derive(Clone, Copy)]
enum Mutation {
    Replace,
    CancelSelect,
    MoveSource,
    MoveOffhand,
    ReplaceSession,
    Remove,
    DeferredReplaceSession,
}

struct DropMutation {
    action: Mutation,
    sword: ItemStack,
    calls: Arc<AtomicUsize>,
}

fn replace_session(player: &Player) {
    let world = player.world();
    let replacement = Arc::new(Player::new(
        player.client.clone(),
        player.gameprofile.clone(),
        PlayerConfig::default(),
        &world,
        pumpkin_util::GameMode::Survival,
    ));
    world.players.rcu(|players| {
        players
            .iter()
            .map(|current| {
                if std::ptr::eq(player, current.as_ref()) {
                    replacement.clone()
                } else {
                    current.clone()
                }
            })
            .collect::<Vec<_>>()
    });
}

impl EventHandler<PlayerDropItemEvent> for DropMutation {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerDropItemEvent,
    ) -> BoxFuture<'a, ()> {
        pumpkin_util::sync::assert_no_guards_held("held-drop live-hand callback");
        assert_eq!(event.item_name, "diamond_spear");
        assert_eq!(event.count, 1);
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.action {
            Mutation::CancelSelect => {
                event.player.inventory.set_selected_slot(4);
                event.cancelled = true;
            }
            action => {
                if matches!(action, Mutation::MoveSource) {
                    let source = event.player.inventory.held_item();
                    event.player.inventory.set_slot(9, source);
                }
                if matches!(action, Mutation::MoveOffhand) {
                    let source = event.player.inventory.held_item();
                    event
                        .player
                        .inventory
                        .set_slot(PlayerInventory::OFF_HAND_SLOT, source);
                    event
                        .player
                        .inventory
                        .set_held_item(ItemStack::EMPTY.clone());
                } else {
                    event.player.inventory.set_held_item(self.sword.clone());
                }
                match action {
                    Mutation::ReplaceSession => replace_session(&event.player),
                    Mutation::Remove => event
                        .player
                        .get_entity()
                        .removal_reason
                        .store(Some(RemovalReason::Discarded)),
                    Mutation::DeferredReplaceSession => {
                        let player = event.player.clone();
                        assert!(guard_effects::defer(move || replace_session(&player)).is_ok());
                    }
                    _ => {}
                }
            }
        }
        Box::pin(async {})
    }
}

fn register_mutation(fixture: &Fixture, action: Mutation, sword: &ItemStack) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerDropItemEvent, _>(
            Arc::new(DropMutation {
                action,
                sword: sword.clone(),
                calls: calls.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    calls
}

fn callback_sword() -> ItemStack {
    let mut sword = weapon(3, &Item::IRON_SWORD);
    sword.set_data_component(UnbreakableImpl);
    sword.set_data_component(CustomNameImpl {
        name: TextComponent::text("callback sword"),
    });
    sword
}

fn real_attack_damage(fixture: &Fixture) -> f32 {
    let player = fixture.player();
    let victim = &fixture.observer.player;
    player.last_attacked_ticks.store(100, Ordering::Relaxed);
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    victim.get_entity().set_pos(Vector3::new(2.0, 64.0, 1.0));
    assert_eq!(player.get_attack_strength_scale(0.5), 1.0);
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert!(!player.get_entity().is_sprinting());
    let before = victim.living_entity.health.load();
    fixture.java.client().handle_attack(
        player,
        &SAttack {
            entity_id: victim.entity_id().into(),
        },
        &fixture.server,
    );
    before - victim.living_entity.health.load()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_callback_replacement_reconciles_live_hand() {
    for action in [
        Mutation::Replace,
        Mutation::CancelSelect,
        Mutation::MoveSource,
    ] {
        let mut fixture = Fixture::new(Route::JavaSingle).await;
        let player = fixture.player().clone();
        let sword = callback_sword();
        fixture.equip(&sword);
        let sword_values = combat_values(&player);
        let spear = weapon(2, &Item::DIAMOND_SPEAR);
        fixture.equip(&spear);
        if matches!(action, Mutation::CancelSelect) {
            player.inventory.set_slot(4, sword.clone());
        }
        let calls = register_mutation(&fixture, action, &sword);
        fixture.drop_packet();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(
            fixture.ground().is_empty(),
            "aborted withdrawal spawned old spear"
        );
        assert_eq!(player.inventory.held_item().uid, sword.uid);
        assert!(player.inventory.held_item().are_equal(&sword));
        if matches!(action, Mutation::CancelSelect) {
            assert_eq!(player.inventory.get_selected_slot(), 4);
            assert_eq!(player.inventory.get_slot(3).uid, spear.uid);
            assert_eq!(player.inventory.get_slot(3).item_count, 2);
        } else if matches!(action, Mutation::MoveSource) {
            assert_eq!(player.inventory.get_slot(9).uid, spear.uid);
            assert_eq!(player.inventory.get_slot(9).item_count, 2);
        }
        assert!(
            fixture.take_equipment().is_empty(),
            "publication preceded tick"
        );
        player.tick(&fixture.server);
        let actual_values = combat_values(&player);
        let updates = fixture.take_equipment();
        let equipment_matches = updates.len() == 1 && updates[0].are_equal(&sword);
        let damage = real_attack_damage(&fixture);
        assert!(
            actual_values == sword_values
                && equipment_matches
                && (damage - sword_values[0] as f32).abs() < 0.00001,
            "stock checks passed; callback hand attributes {actual_values:?}, expected {sword_values:?}; observer equipment matches: {equipment_matches}; actual damage {damage}, expected {}",
            sword_values[0]
        );
        player.tick(&fixture.server);
        assert!(
            fixture.take_equipment().is_empty(),
            "callback published twice"
        );
        assert!(player.inventory.held_item().are_equal(&sword));
        assert!(fixture.ground().is_empty());
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_callback_stock_and_unchanged_controls() {
    for callback in [Callback::Cancel, Callback::Replace] {
        let mut fixture = Fixture::new(Route::JavaSingle).await;
        let spear = weapon(1, &Item::DIAMOND_SPEAR);
        fixture.equip(&spear);
        let player = fixture.player().clone();
        let equipped = combat_values(&player);
        let calls = register_callback(&fixture, callback, 1);
        fixture.drop_packet();
        player.tick(&fixture.server);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(player.inventory.held_item().are_equal(&spear));
        assert_eq!(combat_values(&player), equipped);
        assert!(fixture.ground().is_empty());
        assert!(fixture.take_equipment().is_empty());
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_callback_rejects_stale_and_removed_players() {
    for action in [
        Mutation::ReplaceSession,
        Mutation::Remove,
        Mutation::DeferredReplaceSession,
    ] {
        let mut fixture = Fixture::new(Route::JavaSingle).await;
        let player = fixture.player().clone();
        fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
        let equipped = combat_values(&player);
        let sword = callback_sword();
        let calls = register_mutation(&fixture, action, &sword);
        fixture.drop_packet();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(player.inventory.held_item().are_equal(&sword));
        player.tick(&fixture.server);
        assert_eq!(combat_values(&player), equipped);
        assert!(fixture.ground().is_empty());
        assert!(
            fixture.take_equipment().is_empty(),
            "stale callback published"
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_queue_rejects_retired_player_before_tick_publication() {
    for remove in [false, true] {
        let mut fixture = Fixture::new(Route::JavaSingle).await;
        let player = fixture.player().clone();
        fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
        let equipped = combat_values(&player);
        let sword = callback_sword();
        register_mutation(&fixture, Mutation::Replace, &sword);
        fixture.drop_packet();
        if remove {
            player
                .get_entity()
                .removal_reason
                .store(Some(RemovalReason::Discarded));
        } else {
            replace_session(&player);
        }
        player.tick(&fixture.server);
        assert_eq!(combat_values(&player), equipped);
        assert!(fixture.ground().is_empty());
        assert!(
            fixture.take_equipment().is_empty(),
            "retired queue published"
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_publication_revalidates_after_capture() {
    for remove in [false, true] {
        let mut fixture = Fixture::new(Route::JavaSingle).await;
        let player = fixture.player().clone();
        fixture.equip(&weapon(1, &Item::DIAMOND_SPEAR));
        let equipped = combat_values(&player);
        register_mutation(&fixture, Mutation::Replace, &callback_sword());
        fixture.drop_packet();
        Player::set_equipment_update_test_hook(move |player| {
            if remove {
                player
                    .get_entity()
                    .removal_reason
                    .store(Some(RemovalReason::Discarded));
            } else {
                replace_session(player);
            }
        });
        player.tick(&fixture.server);
        Player::set_equipment_update_test_hook(|_| {});
        assert_eq!(combat_values(&player), equipped);
        assert!(
            fixture.take_equipment().is_empty(),
            "retired capture published"
        );
        fixture.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g07_drop_callback_moved_source_reconciles_both_hands() {
    let mut fixture = Fixture::new(Route::JavaSingle).await;
    let player = fixture.player().clone();
    let baseline = combat_values(&player);
    let spear = weapon(2, &Item::DIAMOND_SPEAR);
    fixture.equip(&spear);
    let calls = register_mutation(&fixture, Mutation::MoveOffhand, &callback_sword());
    fixture.drop_packet();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(player.inventory.held_item().is_empty());
    assert_eq!(player.inventory.off_hand_item().uid, spear.uid);
    assert!(player.inventory.off_hand_item().are_equal(&spear));
    assert!(
        fixture.ground().is_empty(),
        "moved source was dropped again"
    );
    player.tick(&fixture.server);
    assert_eq!(
        combat_values(&player),
        baseline,
        "main-hand spear modifiers survived move"
    );
    let equipment = fixture.take_equipment_slots();
    assert_eq!(equipment.len(), 2);
    assert!(equipment.iter().any(|(slot, stack)| {
        *slot == EquipmentSlot::MAIN_HAND.discriminant() && stack.is_empty()
    }));
    assert!(equipment.iter().any(|(slot, stack)| {
        *slot == EquipmentSlot::OFF_HAND.discriminant() && stack.are_equal(&spear)
    }));
    player.tick(&fixture.server);
    assert!(fixture.take_equipment().is_empty());
    assert!(player.inventory.off_hand_item().are_equal(&spear));
    assert!(fixture.ground().is_empty());
    fixture.close().await;
}
