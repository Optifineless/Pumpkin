#![expect(
    clippy::unwrap_used,
    reason = "Real Java interaction regressions require valid fixtures and callback records"
)]

use super::{b1019_g08_test_support::Fixture, tamable::TamableAnimal, wolf::WolfEntity};
use crate::{
    entity::{Entity, mob::Mob},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_regain_health::EntityRegainHealthEvent,
            player::player_interact_entity_event::PlayerInteractEntityEvent,
        },
    },
    server::Server,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomNameImpl, FoodImpl, MaxStackSizeImpl, UseRemainderImpl},
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_util::{GameMode, Hand, text::TextComponent};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

fn wolf(fixture: &Fixture) -> Arc<WolfEntity> {
    let wolf = WolfEntity::new(Entity::new(
        fixture.world.clone(),
        fixture.player.position(),
        &EntityType::WOLF,
    ));
    TamableAnimal::tame(wolf.as_ref(), fixture.player.gameprofile.id);
    wolf.mob_entity.living_entity.set_max_health(40.0);
    wolf.mob_entity.living_entity.set_health(10.0);
    fixture.world.add_entity_silent(wolf.clone());
    wolf
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_handless_dyeing_uses_the_supplied_stack() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    wolf.set_collar_color(14);
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    let offhand = ItemStack::new(1, &Item::STONE);
    fixture.set_hand(Hand::Right, sword.clone());
    fixture.set_hand(Hand::Left, offhand.clone());
    let mut dye = ItemStack::new(2, &Item::BLUE_DYE);

    assert!(Mob::mob_interact(wolf.as_ref(), &fixture.player, &mut dye));

    assert_eq!(wolf.get_collar_color(), 11);
    assert_eq!(dye.item_count, 1);
    assert!(fixture.hand(Hand::Right).are_equal(&sword));
    assert!(fixture.hand(Hand::Left).are_equal(&offhand));
    fixture.finish().await;
}

fn source_slot(hand: Hand) -> usize {
    if hand == Hand::Right { 0 } else { 40 }
}

fn custom_stew(count: u8) -> (ItemStack, ItemStack) {
    let mut bowl = ItemStack::new(2, &Item::BOWL);
    bowl.set_data_component(CustomNameImpl {
        name: TextComponent::text("Feeding remainder"),
    });
    let mut stew = ItemStack::new(count, &Item::RABBIT_STEW);
    stew.set_data_component(MaxStackSizeImpl { size: 64 });
    stew.set_data_component(UseRemainderImpl {
        remainder: None,
        template: Some(Box::new(bowl.clone())),
    });
    (stew, bowl)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_uses_effective_nutrition_in_both_hands() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    let living = &wolf.mob_entity.living_entity;
    for hand in [Hand::Right, Hand::Left] {
        let steak = ItemStack::new(1, &Item::COOKED_BEEF);
        assert_eq!(steak.get_data_component::<FoodImpl>().unwrap().nutrition, 8);
        let mut custom = steak.clone();
        custom.set_data_component(FoodImpl {
            nutrition: 3,
            saturation: 0.0,
            can_always_eat: false,
        });
        let mut absent = steak.clone();
        absent.remove_data_component(DataComponent::Food);
        let mut zero = steak.clone();
        zero.set_data_component(FoodImpl {
            nutrition: 0,
            saturation: 0.0,
            can_always_eat: false,
        });
        for (food, before, expected) in [
            (steak.clone(), 10.0, 26.0),
            (steak, 39.0, 40.0),
            (custom, 10.0, 16.0),
            (absent, 10.0, 12.0),
            (zero, 10.0, 10.0),
        ] {
            living.set_health(before);
            fixture.set_hand(hand, food);
            fixture.interact(wolf.as_ref(), hand);
            assert_eq!(living.health.load(), expected, "wolf nutrition healing");
            assert!(fixture.hand(hand).is_empty(), "one food must be paid");
        }
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_nonfood_and_bones_do_not_heal_but_wild_bones_tame() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    for hand in [Hand::Right, Hand::Left] {
        for item in [&Item::STICK, &Item::BONE] {
            let input = ItemStack::new(2, item);
            fixture.set_hand(hand, input.clone());
            fixture.interact(wolf.as_ref(), hand);
            assert_eq!(wolf.mob_entity.living_entity.health.load(), 10.0);
            assert!(fixture.hand(hand).are_equal(&input));
        }
        wolf.set_tame(false);
        wolf.set_owner(None);
        fixture.set_hand(hand, ItemStack::new(2, &Item::BONE));
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(
            fixture.hand(hand).item_count,
            1,
            "wild taming pays one bone"
        );
        assert_eq!(wolf.mob_entity.living_entity.health.load(), 10.0);
        TamableAnimal::tame(wolf.as_ref(), fixture.player.gameprofile.id);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_stew_preserves_component_remainders_and_creative_input() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    for hand in [Hand::Right, Hand::Left] {
        let living = &wolf.mob_entity.living_entity;
        living.set_health(10.0);
        fixture.set_hand(hand, ItemStack::new(1, &Item::RABBIT_STEW));
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(living.health.load(), 30.0, "stew nutrition is ten");
        assert_eq!(fixture.hand(hand).item, &Item::BOWL);
        assert_eq!(fixture.hand(hand).item_count, 1);

        let (stew, bowl) = custom_stew(1);
        living.set_health(10.0);
        fixture.set_hand(hand, stew);
        fixture.interact(wolf.as_ref(), hand);
        assert!(fixture.hand(hand).are_equal(&bowl));

        let before_bowls = fixture.inventory_count(&Item::BOWL);
        let (stew, _) = custom_stew(2);
        living.set_health(10.0);
        fixture.set_hand(hand, stew.clone());
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(fixture.hand(hand).item_count, 1);
        assert!(fixture.hand(hand).are_items_and_components_equal(&stew));
        // Replacing the held bowl removed two bowls before inserting this remainder.
        assert_eq!(fixture.inventory_count(&Item::BOWL), before_bowls);
        let bearing_bowls: u32 = (0..41)
            .map(|slot| fixture.player.inventory().get_stack(slot))
            .filter(|stack| stack.are_items_and_components_equal(&bowl))
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert!(
            bearing_bowls >= 2,
            "remainder retains its component template"
        );

        fixture.player.gamemode.store(GameMode::Creative);
        living.set_health(10.0);
        fixture.set_hand(hand, stew.clone());
        let before_bowls = fixture.inventory_count(&Item::BOWL);
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(living.health.load(), 30.0);
        assert!(fixture.hand(hand).are_equal(&stew));
        assert_eq!(fixture.inventory_count(&Item::BOWL), before_bowls);
        fixture.player.gamemode.store(GameMode::Survival);
    }
    fixture.finish().await;
}

struct RegainCallback<F>(F);

impl<F: Fn(&mut EntityRegainHealthEvent) + Send + Sync> EventHandler<EntityRegainHealthEvent>
    for RegainCallback<F>
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityRegainHealthEvent,
    ) -> BoxFuture<'a, ()> {
        (self.0)(event);
        Box::pin(async {})
    }
}

#[derive(Clone, Copy, Debug)]
enum CallbackEdit {
    Cancel,
    Replace,
    RestoreOriginal,
    Remove,
    Select,
}

async fn callback_edit(hand: Hand, edit: CallbackEdit) {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    let input = ItemStack::new(2, &Item::COOKED_BEEF);
    fixture.set_hand(hand, input.clone());
    let other = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.player.inventory().set_stack(1, other.clone());
    let player = fixture.player.clone();
    let records = Arc::new(Mutex::new(Vec::new()));
    let seen = records.clone();
    let replacement = ItemStack::new(3, &Item::GOLDEN_CARROT);
    let callback_replacement = replacement.clone();
    let callback_original = input.clone();
    fixture
        .server
        .plugin_manager
        .register::<EntityRegainHealthEvent, _>(
            Arc::new(RegainCallback(
                move |event: &mut EntityRegainHealthEvent| {
                    let source = source_slot(hand);
                    let observed = player.inventory().get_stack(source);
                    seen.lock().unwrap().push((event.amount, observed));
                    match edit {
                        CallbackEdit::Cancel => event.cancelled = true,
                        CallbackEdit::Replace => {
                            player
                                .inventory()
                                .set_stack(source, callback_replacement.clone());
                        }
                        CallbackEdit::RestoreOriginal => {
                            player
                                .inventory()
                                .set_stack(source, callback_original.clone());
                        }
                        CallbackEdit::Remove => {
                            player
                                .inventory()
                                .set_stack(source, ItemStack::EMPTY.clone());
                        }
                        CallbackEdit::Select => player.inventory().set_selected_slot(1),
                    }
                },
            )),
            EventPriority::Normal,
            true,
        );
    fixture.interact(wolf.as_ref(), hand);
    let records = records.lock().unwrap().clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].0, 16.0);
    assert_eq!(
        records[0].1.item_count, 1,
        "regain callback sees committed payment"
    );
    assert_eq!(records[0].1.item, &Item::COOKED_BEEF);
    assert_eq!(
        wolf.mob_entity.living_entity.health.load(),
        if matches!(edit, CallbackEdit::Cancel) {
            10.0
        } else {
            26.0
        }
    );
    let current = fixture.player.inventory().get_stack(source_slot(hand));
    match edit {
        CallbackEdit::Replace => assert!(current.are_equal(&replacement)),
        CallbackEdit::RestoreOriginal => {
            assert!(current.are_equal(&input));
            assert_eq!(current.uid, input.uid);
        }
        CallbackEdit::Remove => assert!(current.is_empty()),
        CallbackEdit::Cancel | CallbackEdit::Select => assert_eq!(current.item_count, 1),
    }
    assert!(fixture.player.inventory().get_stack(1).are_equal(&other));
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_regain_callbacks_see_payment_and_retain_inventory_edits() {
    for hand in [Hand::Right, Hand::Left] {
        for edit in [
            CallbackEdit::Cancel,
            CallbackEdit::Replace,
            CallbackEdit::RestoreOriginal,
            CallbackEdit::Remove,
            CallbackEdit::Select,
        ] {
            callback_edit(hand, edit).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_stew_overflow_is_committed_before_unlocked_heal_callback() {
    for hand in [Hand::Right, Hand::Left] {
        let fixture = Fixture::new();
        let wolf = wolf(&fixture);
        for slot in 0..36 {
            fixture
                .player
                .inventory()
                .set_stack(slot, ItemStack::new(64, &Item::STONE));
        }
        let (stew, _) = custom_stew(2);
        fixture.set_hand(hand, stew);
        let player = fixture.player.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let records = seen.clone();
        fixture
            .server
            .plugin_manager
            .register::<EntityRegainHealthEvent, _>(
                Arc::new(RegainCallback(move |_: &mut EntityRegainHealthEvent| {
                    let inventory = player.inventory().clone();
                    records
                        .lock()
                        .unwrap()
                        .push(inventory.get_stack(source_slot(hand)).item_count);
                    let (done, receive) = mpsc::channel();
                    std::thread::spawn(move || {
                        inventory.set_stack(2, ItemStack::new(1, &Item::DIAMOND));
                        let _ = done.send(());
                    });
                    assert!(
                        receive.recv_timeout(Duration::from_secs(5)).is_ok(),
                        "inventory owner must be released before regain callbacks"
                    );
                })),
                EventPriority::Normal,
                true,
            );
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(*seen.lock().unwrap(), [1]);
        assert_eq!(wolf.mob_entity.living_entity.health.load(), 30.0);
        assert_eq!(fixture.dropped_count(&Item::BOWL), 2);
        assert_eq!(fixture.inventory_count(&Item::BOWL), 0);
        assert_eq!(fixture.player.inventory().get_stack(2).item, &Item::DIAMOND);
        fixture.finish().await;
    }
}

struct CancelInteraction;

impl EventHandler<PlayerInteractEntityEvent> for CancelInteraction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_cancelled_interaction_keeps_food_and_health() {
    let fixture = Fixture::new();
    let wolf = wolf(&fixture);
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(
            Arc::new(CancelInteraction),
            EventPriority::Normal,
            true,
        );
    for hand in [Hand::Right, Hand::Left] {
        let food = ItemStack::new(2, &Item::COOKED_BEEF);
        fixture.set_hand(hand, food.clone());
        fixture.interact(wolf.as_ref(), hand);
        assert_eq!(wolf.mob_entity.living_entity.health.load(), 10.0);
        assert!(fixture.hand(hand).are_equal(&food));
    }
    fixture.finish().await;
}

struct PreInteractionEdit {
    slot: usize,
    edit: CallbackEdit,
}

impl EventHandler<PlayerInteractEntityEvent> for PreInteractionEdit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        let inventory = event.player.inventory();
        match self.edit {
            CallbackEdit::Replace => {
                inventory.set_stack(self.slot, ItemStack::new(2, &Item::COOKED_BEEF));
            }
            CallbackEdit::Remove => inventory.set_stack(self.slot, ItemStack::EMPTY.clone()),
            CallbackEdit::Select => inventory.set_selected_slot(1),
            CallbackEdit::Cancel => event.cancelled = true,
            CallbackEdit::RestoreOriginal => {}
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g08_wolf_preinteraction_edits_validate_the_originating_hand() {
    for hand in [Hand::Right, Hand::Left] {
        for edit in [
            CallbackEdit::Replace,
            CallbackEdit::Remove,
            CallbackEdit::Select,
        ] {
            let fixture = Fixture::new();
            let wolf = wolf(&fixture);
            fixture.set_hand(hand, ItemStack::new(2, &Item::COOKED_BEEF));
            let other = ItemStack::new(1, &Item::DIAMOND_SWORD);
            fixture.player.inventory().set_stack(1, other.clone());
            fixture
                .server
                .plugin_manager
                .register::<PlayerInteractEntityEvent, _>(
                    Arc::new(PreInteractionEdit {
                        slot: source_slot(hand),
                        edit,
                    }),
                    EventPriority::Normal,
                    true,
                );
            fixture.interact(wolf.as_ref(), hand);
            let admitted = hand == Hand::Left && matches!(edit, CallbackEdit::Select);
            assert_eq!(
                wolf.mob_entity.living_entity.health.load(),
                if admitted { 26.0 } else { 10.0 }
            );
            let source = fixture.player.inventory().get_stack(source_slot(hand));
            if matches!(edit, CallbackEdit::Remove) {
                assert!(source.is_empty());
            } else {
                assert_eq!(source.item_count, if admitted { 1 } else { 2 });
            }
            assert!(fixture.player.inventory().get_stack(1).are_equal(&other));
            fixture.finish().await;
        }
    }
}
