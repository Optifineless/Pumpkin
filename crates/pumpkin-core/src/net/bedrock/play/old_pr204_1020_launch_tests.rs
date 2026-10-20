use super::*;
use crate::old_pr204_1020_tests::support::{Hooks, Veto, charge, loose, stack, used};
use pumpkin_protocol::bedrock::server::inventory_transaction::ReleaseItemTransactionData;
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

fn send_use(f: &HandUseFixture, hand: Hand, block: bool) {
    let player = &f.owner.player;
    let held = player.inventory().get_stack_in_hand(hand);
    let mut packet = click_block_packet(
        player,
        if hand == Hand::Right {
            HandSlot::Mainhand
        } else {
            HandSlot::Offhand
        },
        BlockPos::new(8, 63, 8),
        &held,
        Vec::new(),
    );
    if let TransactionData::UseItem(data) = &mut packet.transaction_data {
        data.action_type = VarInt(i32::from(!block));
    }
    f.owner.client().handle_inventory_action(player, packet);
}
fn release(f: &HandUseFixture, hand: Hand) {
    let player = &f.owner.player;
    let held = player.inventory().get_stack_in_hand(hand);
    player
        .living_entity
        .item_use_time
        .store(72000 - 30, std::sync::atomic::Ordering::SeqCst);
    let mut packet = click_block_packet(
        player,
        if hand == Hand::Right {
            HandSlot::Mainhand
        } else {
            HandSlot::Offhand
        },
        BlockPos::new(8, 63, 8),
        &held,
        Vec::new(),
    );
    packet.transaction_type = VarUInt(4);
    packet.transaction_data = TransactionData::ReleaseItem(ReleaseItemTransactionData {
        action_type: VarInt(0),
        hot_bar_slot: VarInt(i32::from(player.inventory().get_selected_slot())),
        item_in_hand: NetworkItemDescriptor::from(&held),
        head_position: Vector3::new(8.5, 64.0, 8.5),
    });
    f.owner.client().handle_inventory_action(player, packet);
}
fn case(
    fixture: &mut HandUseFixture,
    hooks: &Arc<Hooks>,
    hand: Hand,
    route: usize,
    veto: Veto,
    edit: i32,
) -> TestResult {
    let player = fixture.owner.player.clone();
    let world = fixture.world.world();
    pumpkin_inventory::inventory::slot_transaction::clear(player.inventory().as_ref());
    player.inventory().set_selected_slot(4);
    player.living_entity.clear_active_hand();
    if player.get_entity().is_fall_flying() {
        player.get_entity().set_fall_flying(false);
    }
    let item = match route {
        0 => &Item::EGG,
        1 | 2 => &Item::FIREWORK_ROCKET,
        3 => &Item::BOW,
        4 => &Item::CROSSBOW,
        _ => &Item::TRIDENT,
    };
    let original = stack(1, item);
    player.inventory().set_stack_in_hand(hand, original.clone());
    let source = if hand == Hand::Right { 4 } else { 40 };
    if route == 2 {
        // This fixture starts in admitted flight; #253 owns start/tick admission.
        player
            .inventory()
            .set_stack(38, ItemStack::new(1, &Item::ELYTRA));
        player
            .get_entity()
            .on_ground
            .store(false, std::sync::atomic::Ordering::SeqCst);
        player.get_entity().set_fall_flying(true);
    }
    if route == 3 || route == 4 {
        player.inventory().set_stack(9, stack(2, &Item::ARROW));
    }
    hooks.configure(
        veto,
        false,
        Some(Arc::new(move |p| {
            if edit == 1 {
                p.inventory().set_stack(source, stack(1, &Item::DIAMOND));
            }
            if edit == 2 {
                p.inventory().set_selected_slot(5);
                p.inventory().set_stack(5, stack(1, &Item::DIAMOND));
            }
        })),
    );
    fixture.drain_packets();
    let before = world.entities.load().len();
    let used_before = used(&player, item);
    send_use(fixture, hand, route == 1);
    if route >= 3 {
        release(fixture, hand);
    }
    if route == 4 {
        send_use(fixture, hand, false);
    }
    let admitted = veto == Veto::None;
    assert_eq!(
        used(&player, item) - used_before,
        i32::from(admitted),
        "Bedrock veto/success use statistic for route {route}"
    );
    assert_eq!(
        hooks.counts().0,
        1,
        "Bedrock route {route} did not reach launch admission"
    );
    assert_eq!(world.entities.load().len() - before, usize::from(admitted));
    let actual = player.inventory().get_stack(source);
    if !admitted && edit == 0 {
        assert_eq!(actual.uid, original.uid);
        assert!(actual.are_equal(&original));
    }
    if !admitted && edit == 1 {
        assert_eq!(actual.item, &Item::DIAMOND);
    }
    if route == 0 || route == 1 || route == 2 || route == 5 {
        assert_eq!(loose(&player, item), usize::from(!admitted));
    }
    if route == 3 {
        assert_eq!(loose(&player, &Item::ARROW), 2 - usize::from(admitted));
    }
    if route == 4 {
        assert_eq!(charge(&actual), 0);
        assert_eq!(loose(&player, &Item::ARROW), 2 - usize::from(admitted));
    }
    player.tick(&fixture.world.server);
    let packets = fixture.owner.take_packets();
    let (window, index, _) = hand_address(hand);
    assert_owner_inventory_slot(&packets, window, index, &actual, false)?;
    assert_live_equipment(fixture, hand, &packets)
}
fn matrix(fixture: &mut HandUseFixture, hooks: &Arc<Hooks>) -> Result<(), String> {
    for hand in [Hand::Right, Hand::Left] {
        for route in 0..6 {
            for veto in [Veto::Launch, Veto::Spawn, Veto::None] {
                for edit in 0..3 {
                    case(fixture, hooks, hand, route, veto, edit).map_err(|err| err.to_string())?;
                }
            }
        }
    }
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr204_1020_bedrock_veto_packets_keep_live_hand() -> TestResult {
    let mut fixture = tokio::time::timeout(Duration::from_secs(90), HandUseFixture::new()).await?;
    let player = fixture.owner.player.clone();
    let world = fixture.world.world();
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    world.set_block_state(
        &BlockPos::new(8, 63, 8),
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let hooks = Hooks::install(&fixture.world.server, &player);
    let runtime = tokio::runtime::Handle::current();
    let (send, recv) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            matrix(&mut fixture, &hooks)
        }));
        let _ = send.send((fixture, result));
    });
    let (fixture, result) = recv.recv_timeout(Duration::from_secs(90))?;
    tokio::time::timeout(Duration::from_secs(30), fixture.close()).await?;
    match result {
        Ok(result) => result.map_err(Into::into),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn assert_live_equipment(
    fixture: &mut HandUseFixture,
    hand: Hand,
    owner: &[bytes::Bytes],
) -> TestResult {
    use pumpkin_protocol::ser::NetworkReadExt;
    let expected = fixture.owner.player.inventory().get_stack_in_hand(hand);
    let mut latest = None;
    for packet in fixture.java_observer.take_packets() {
        let mut data = packet.as_ref();
        if data.get_var_int()?.0 != pumpkin_data::packet::clientbound::play::SET_EQUIPMENT.0
            || data.get_var_int()?.0 != fixture.owner.player.entity_id()
        {
            continue;
        }
        for _ in 0..8 {
            let slot = data.get_u8()?;
            let stack = ItemStackSerializer::read(&mut data)?.0.into_owned();
            if slot & 127 == equipment_slot(hand).discriminant() as u8 {
                latest = Some(stack);
            }
            if slot & 128 == 0 {
                break;
            }
        }
    }
    if let Some(actual) = latest {
        assert!(
            actual.are_equal(&expected),
            "Java observer retained stale Bedrock hand"
        );
    } else {
        assert!(
            !fixture.hand_changed(hand),
            "Java observer missed changed Bedrock hand"
        );
    }
    assert_bedrock_equipment(fixture, hand, &expected, owner, true)?;
    let observer = fixture.bedrock_observer.take_packets();
    assert_bedrock_equipment(fixture, hand, &expected, &observer, false)
}
fn assert_bedrock_equipment(
    fixture: &HandUseFixture,
    hand: Hand,
    expected: &ItemStack,
    packets: &[bytes::Bytes],
    allow_missing: bool,
) -> TestResult {
    use crate::net::bedrock::play::inventory_exploit_tests::packet_payload;
    use pumpkin_protocol::{codec::var_ulong::VarULong, serial::PacketRead};
    let (container, _, _) = hand_address(hand);
    let mut latest = None;
    for packet in packets {
        if let Some(mut data) = packet_payload(packet, 31) {
            if VarULong::read(&mut data)?.0 != fixture.owner.player.entity_id() as u64 {
                continue;
            }
            let item = NetworkItemStackDescriptor::read(&mut data)?;
            let slot = u8::read(&mut data)?;
            let selected = u8::read(&mut data)?;
            let window = u8::read(&mut data)?;
            if i32::from(window) == container {
                latest = Some((item, slot, selected));
            }
        }
    }
    if let Some((actual, slot, selected)) = latest {
        let expected = NetworkItemStackDescriptor::from(expected);
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.stack_size, expected.stack_size);
        assert_eq!(actual.extra_data, expected.extra_data);
        assert_eq!(
            selected,
            fixture.owner.player.inventory().get_selected_slot()
        );
        assert_eq!(slot, if hand == Hand::Right { selected } else { 0 });
    } else {
        assert!(
            allow_missing || !fixture.hand_changed(hand),
            "Bedrock equipment correction missing"
        );
    }
    Ok(())
}
