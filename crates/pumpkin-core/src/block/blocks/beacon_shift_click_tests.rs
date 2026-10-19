use super::*;
use pumpkin_data::data_component_impl::CustomNameImpl;
use pumpkin_inventory::{
    player::player_inventory::PlayerInventory, screen_handler::InventoryPlayer, slot::Slot,
};
use pumpkin_util::text::TextComponent;
use std::time::Duration;

fn named_stack(count: u8, item: &'static Item, name: &str) -> ItemStack {
    let mut stack = ItemStack::new(count, item);
    stack.set_data_component(CustomNameImpl {
        name: TextComponent::text(name.to_owned()),
    });
    stack
}

fn reset_inventory(fixture: &Fixture) {
    for index in 0..PlayerInventory::MAIN_SIZE {
        fixture
            .player
            .player
            .inventory
            .set_stack(index, ItemStack::EMPTY.clone());
    }
    beacon_payment(&fixture.player.player).remove_stack(0);
}

fn assert_inventory(fixture: &Fixture, expected: &[(usize, ItemStack)]) {
    for index in 0..PlayerInventory::MAIN_SIZE {
        let actual = fixture.player.player.inventory.get_stack(index);
        let wanted = expected.iter().find(|(slot, _)| *slot == index);
        if let Some((_, wanted)) = wanted {
            assert!(
                actual.are_equal(wanted),
                "inventory slot {index} should preserve item, count and components"
            );
        } else {
            assert!(
                actual.is_empty(),
                "unexpected stock in inventory slot {index}"
            );
        }
    }
}

fn assert_fallback(
    fixture: &Fixture,
    source: usize,
    menu_slot: i16,
    destination: usize,
    stack: ItemStack,
    occupied_payment: bool,
) {
    reset_inventory(fixture);
    fixture
        .player
        .player
        .inventory
        .set_stack(source, stack.clone());
    let payment = beacon_payment(&fixture.player.player);
    let unchanged_payment = if occupied_payment {
        named_stack(1, &Item::EMERALD, "already in payment")
    } else {
        ItemStack::EMPTY.clone()
    };
    payment.set_stack(0, unchanged_payment.clone());
    click(
        &fixture.player.player,
        &fixture.server,
        menu_slot,
        0,
        SlotActionType::QuickMove,
    );
    assert!(
        fixture.player.player.inventory.get_stack(source).is_empty(),
        "beacon shift-click fallback must empty source slot {source}"
    );
    assert_inventory(fixture, &[(destination, stack)]);
    assert!(payment.get_stack(0).are_equal(&unchanged_payment));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_shift_click_falls_back() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = Fixture::new(0);
        fixture.use_beacon();
        for (source, menu_slot, destination, count, item, occupied) in [
            (9, 1, 0, 5, &Item::STONE, false),
            (0, 28, 9, 7, &Item::STONE, false),
            (0, 28, 9, 2, &Item::DIAMOND, false),
            (9, 1, 0, 1, &Item::DIAMOND, true),
        ] {
            assert_fallback(
                &fixture,
                source,
                menu_slot,
                destination,
                named_stack(count, item, "fallback component"),
                occupied,
            );
        }
        fixture.finish().await;
    })
    .await
    .expect("beacon shift-click regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_singleton_deposit_control() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = Fixture::new(0);
        fixture.use_beacon();
        let singleton = named_stack(1, &Item::EMERALD, "single payment component");
        fixture
            .player
            .player
            .inventory
            .set_stack(9, singleton.clone());
        click(
            &fixture.player.player,
            &fixture.server,
            1,
            0,
            SlotActionType::QuickMove,
        );
        assert_inventory(&fixture, &[]);
        let payment = beacon_payment(&fixture.player.player);
        assert!(payment.get_stack(0).are_equal(&singleton));
        click(
            &fixture.player.player,
            &fixture.server,
            0,
            0,
            SlotActionType::QuickMove,
        );
        assert!(payment.get_stack(0).is_empty());
        assert_inventory(&fixture, &[(8, singleton)]);
        fixture.finish().await;
    })
    .await
    .expect("beacon singleton control timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_shift_click_full_destinations() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = Fixture::new(0);
        fixture.use_beacon();
        for (source, menu_slot, full_range) in [(9, 1, 0..9), (0, 28, 9..36)] {
            reset_inventory(&fixture);
            let stack = named_stack(5, &Item::STONE, "full destination component");
            let mut expected = vec![(source, stack.clone())];
            fixture.player.player.inventory.set_stack(source, stack);
            for index in full_range {
                let full = ItemStack::new(64, &Item::DIRT);
                fixture
                    .player
                    .player
                    .inventory
                    .set_stack(index, full.clone());
                expected.push((index, full));
            }
            click(
                &fixture.player.player,
                &fixture.server,
                menu_slot,
                0,
                SlotActionType::QuickMove,
            );
            assert_inventory(&fixture, &expected);
            assert!(
                beacon_payment(&fixture.player.player)
                    .get_stack(0)
                    .is_empty()
            );
        }
        fixture.finish().await;
    })
    .await
    .expect("beacon full-destination control timed out");
}

struct UntakeableSlot(Arc<dyn Slot>);

impl Slot for UntakeableSlot {
    fn get_inventory(&self) -> Arc<dyn Inventory> {
        self.0.get_inventory()
    }

    fn get_index(&self) -> usize {
        self.0.get_index()
    }

    fn set_id(&self, id: usize) {
        self.0.set_id(id);
    }

    fn mark_dirty(&self) {
        self.0.mark_dirty();
    }

    fn can_take_items(&self, _player: &dyn InventoryPlayer) -> bool {
        false
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_shift_click_take_control() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = Fixture::new(0);
        fixture.use_beacon();
        let screen = current_screen(&fixture.player.player);
        let mut handler = screen.lock().unwrap();
        let original = handler.get_behaviour().slots[1].clone();
        handler.get_behaviour_mut().slots[1] = Arc::new(UntakeableSlot(original));
        drop(handler);
        for item in [&Item::EMERALD, &Item::STONE] {
            let source = named_stack(1, item, "take predicate component");
            fixture.player.player.inventory.set_stack(9, source.clone());
            click(
                &fixture.player.player,
                &fixture.server,
                1,
                0,
                SlotActionType::QuickMove,
            );
            assert_inventory(&fixture, &[(9, source)]);
            assert!(
                beacon_payment(&fixture.player.player)
                    .get_stack(0)
                    .is_empty()
            );
        }
        fixture.finish().await;
    })
    .await
    .expect("beacon take-predicate control timed out");
}
