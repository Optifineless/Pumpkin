use super::*;
use pumpkin_data::{
    data_component_impl::{CustomDataImpl, CustomNameImpl},
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    ClientPacket, MultiVersionJavaPacket, RawPacket, java::server::play::SCloseContainer,
};
use pumpkin_util::text::TextComponent;
use std::time::Duration;

fn ground_payments(world: &World) -> Vec<ItemStack> {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| entity.get_item_entity())
        .map(|item| item.get_item_stack().lock().unwrap().clone())
        .filter(|stack| stack.item == &Item::EMERALD)
        .collect()
}

pub(super) fn ground_emerald_count(world: &World) -> u32 {
    ground_payments(world)
        .iter()
        .map(|stack| u32::from(stack.item_count))
        .sum()
}

fn close_packet(fixture: &Fixture, sync_id: u8) {
    let mut payload = Vec::new();
    SCloseContainer {
        window_id: VarInt(i32::from(sync_id)),
    }
    .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
    .unwrap();
    fixture
        .player
        .client()
        .handle_play_packet(
            &fixture.player.player,
            &fixture.server,
            &RawPacket {
                id: SCloseContainer::to_id(CURRENT_MC_VERSION),
                payload: payload.into(),
            },
        )
        .unwrap();
    assert!(!fixture.player.client().is_closed());
}

async fn close_with_inventory(inventory_full: bool) {
    let fixture = Fixture::new(1);
    fixture.use_beacon();
    let mut expected = ItemStack::new(1, &Item::EMERALD);
    expected.set_data_component(CustomNameImpl {
        name: TextComponent::text("Unused beacon payment"),
    });
    let mut data = NbtCompound::new();
    data.put_int("beacon_close_marker", 285);
    expected.set_data_component(CustomDataImpl::new(data));
    fixture
        .player
        .player
        .inventory
        .set_stack(0, expected.clone());
    click(
        &fixture.player.player,
        &fixture.server,
        28,
        0,
        SlotActionType::QuickMove,
    );
    let payment = beacon_payment(&fixture.player.player);
    assert!(payment.get_stack(0).are_equal(&expected));
    assert_eq!(emerald_count(&fixture.player.player), 0);
    if inventory_full {
        for slot in 0..PlayerInventory::MAIN_SIZE {
            fixture
                .player
                .player
                .inventory
                .set_stack(slot, ItemStack::new(64, &Item::STONE));
        }
    }
    let sync_id = menu_sync_id(&fixture.player.player);
    close_packet(&fixture, sync_id);

    assert!(payment.get_stack(0).is_empty());
    assert_eq!(
        emerald_count(&fixture.player.player) + ground_emerald_count(&fixture.world),
        1,
        "closing must conserve the payment with inventory_full={inventory_full}"
    );
    let dropped = ground_payments(&fixture.world);
    assert_eq!(
        dropped.len(),
        1,
        "unused payment must be on the ground with inventory_full={inventory_full}"
    );
    assert!(dropped[0].are_equal(&expected));
    assert_eq!(emerald_count(&fixture.player.player), 0);

    close_packet(&fixture, sync_id);
    fixture.player.player.tick(&fixture.server);
    let after_tick = ground_payments(&fixture.world);
    assert_eq!(
        after_tick.len(),
        1,
        "repeated close must not duplicate payment"
    );
    assert!(after_tick[0].are_equal(&expected));
    assert!(payment.get_stack(0).is_empty());
    assert_eq!(emerald_count(&fixture.player.player), 0);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_close_drops_unused_payment_once() {
    tokio::time::timeout(Duration::from_secs(30), async {
        // Full inventory also drops on the unfixed base; leave that control first.
        close_with_inventory(true).await;
        close_with_inventory(false).await;
    })
    .await
    .expect("bounded beacon close regression timed out");
}
