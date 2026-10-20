use crate::{entity::death_test_world::DeathTestWorld, net::java::combat_test_support::TestPlayer};
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{Inventory, screen_handler::ScreenHandler};
use pumpkin_protocol::{VarInt, java::server::play::SlotActionType};
use pumpkin_util::GameMode;

async fn armor_click(action: SlotActionType) {
    let world = DeathTestWorld::new().await;
    for (mode, cursed) in [
        (GameMode::Survival, true),
        (GameMode::Adventure, true),
        (GameMode::Survival, false),
        (GameMode::Creative, true),
    ] {
        let fixture = TestPlayer::new(&world.world());
        fixture.player.set_gamemode(mode);
        let mut helmet = ItemStack::new(1, &Item::DIAMOND_HELMET);
        if cursed {
            helmet.add_enchantment(&Enchantment::BINDING_CURSE, 1);
        }
        fixture.player.inventory.set_stack(39, helmet.clone());
        let mut packet = crate::inventory_transaction_tests::click();
        packet.sync_id = VarInt(0);
        packet.slot = 5;
        packet.mode = action.clone();
        fixture.player.on_slot_click(packet, &world.server);
        let equipped = fixture.player.inventory.get_stack(39);
        let handler = fixture.player.player_screen_handler.lock().unwrap();
        let cursor = handler.get_behaviour().cursor_stack.lock().unwrap();
        let blocked = cursed && mode != GameMode::Creative;
        if blocked {
            assert!(equipped.are_equal(&helmet), "{mode:?} removed cursed armor");
            assert_eq!(equipped.uid, helmet.uid);
            assert!(cursor.is_empty());
            assert!((0..36).all(|slot| fixture.player.inventory.get_stack(slot).is_empty()));
        } else {
            assert!(equipped.is_empty());
            if action == SlotActionType::Pickup {
                assert!(cursor.are_equal(&helmet));
            } else {
                assert!(fixture.player.inventory.get_stack(9).are_equal(&helmet));
                assert!(cursor.is_empty());
            }
        }
    }
    world.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_pickup_rejects_binding_cursed_armor() {
    armor_click(SlotActionType::Pickup).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn java_shift_click_rejects_binding_cursed_armor() {
    armor_click(SlotActionType::QuickMove).await;
}
