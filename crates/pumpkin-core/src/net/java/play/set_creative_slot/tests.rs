use super::*;
use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{ClientPacket, RawPacket, packet::MultiVersionJavaPacket};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn survival_creative_slot_packet_is_ignored_without_disconnecting() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let mut fixture = TestPlayer::new(&world);
    let before = ItemStack::new(3, &Item::STONE);
    fixture.player.inventory().set_stack(0, before);
    fixture.player.set_gamemode(GameMode::Creative);
    fixture.player.set_gamemode(GameMode::Survival);
    fixture.take_packets();
    let inventory: Vec<_> = (0..fixture.player.inventory().size())
        .map(|slot| fixture.player.inventory().get_stack(slot))
        .collect();

    for slot in [36, -1] {
        let packet = SSetCreativeSlot::new(slot, ItemStack::new(64, &Item::DIAMOND).into());
        let mut payload = Vec::new();
        packet
            .write_packet_data(&mut payload, &pumpkin_data::packet::CURRENT_MC_VERSION)
            .unwrap();
        fixture.player.inbound_packets.push(RawPacket {
            id: SSetCreativeSlot::to_id(pumpkin_data::packet::CURRENT_MC_VERSION),
            payload: payload.into(),
        });
        // The real tick packet queue applies the error-to-kick policy, not just the handler.
        fixture.player.process_inbound_packets();
        assert!(!fixture.client().is_closed());
        for (index, stack) in inventory.iter().enumerate() {
            assert!(fixture.player.inventory().get_stack(index).are_equal(stack));
        }
        assert!(world.entities.load().is_empty());
        assert!(fixture.take_packets().is_empty());
    }
}
