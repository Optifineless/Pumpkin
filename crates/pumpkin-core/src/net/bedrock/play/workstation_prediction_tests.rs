#![expect(
    clippy::unwrap_used,
    clippy::unreachable,
    reason = "Real workstation request regressions"
)]

use std::sync::Arc;

use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    bedrock::{
        client::inventory_content::CInventoryContent,
        network_item::{
            ContainerName, FullContainerName, ItemStackWrapper, NetworkItemStackDescriptor,
        },
        server::item_stack_request::{
            ItemStackRequest, ItemStackRequestAction as Action, SItemStackRequest,
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    packet::Packet,
    serial::{PacketRead, PacketWrite},
};

use super::{
    inventory_exploit_tests::packet_payload,
    inventory_request_beacon_tests::support::{Fixture, selection},
    loom_workstation_tests::{
        banner,
        support::{LoomFixture, ResponseSlot, responses, slot},
    },
};
use crate::{
    entity::EntityBase,
    net::{
        bedrock::combat_test_support::TestBedrockPlayer,
        java::{combat_test_support::TestPlayer, sound_test_support::decode_sounds},
    },
};

fn request(id: i32, actions: Vec<Action>) -> ItemStackRequest {
    ItemStackRequest {
        request_id: VarInt(id),
        actions,
        filter_strings: Vec::new(),
        filter_cause: 0,
    }
}

fn send(owner: &TestBedrockPlayer, requests: Vec<ItemStackRequest>) {
    owner
        .client()
        .handle_item_stack_request(&owner.player, SItemStackRequest { requests });
}

fn payment_move(fixture: &Fixture) -> Action {
    let payment = ItemStack::new(2, &Item::EMERALD);
    fixture
        .owner
        .player
        .inventory()
        .set_stack(9, payment.clone());
    Action::Place {
        count: 1,
        source: slot(ContainerName::Inventory, 9, &payment),
        destination: slot(ContainerName::BeaconPayment, 27, ItemStack::EMPTY),
    }
}

fn confirm(reference: i32) -> Vec<Action> {
    let mut source = slot(ContainerName::BeaconPayment, 27, ItemStack::EMPTY);
    source.stack_id = reference;
    vec![selection(1, 0), Action::Destroy { count: 1, source }]
}

fn paid_response(updates: &[ResponseSlot]) {
    assert_eq!(
        updates,
        [ResponseSlot {
            name: ContainerName::BeaconPayment,
            slot: 27,
            count: 0,
            uid: 0
        }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_beacon_unacknowledged_prediction() {
    let mut fixture = Fixture::new(1).await;
    fixture.owner.take_packets();
    let action = payment_move(&fixture);
    send(&fixture.owner, vec![request(-1, vec![action])]);
    send(&fixture.owner, vec![request(-3, confirm(-1))]);
    // Do not consume either queued response until both requests have committed.
    let decoded = responses(&fixture.owner.take_packets());
    assert_eq!(
        decoded
            .iter()
            .map(|(id, status, _)| (*id, *status))
            .collect::<Vec<_>>(),
        [(-1, 0), (-3, 0)]
    );
    paid_response(&decoded[1].2);
    assert_eq!(fixture.owner.player.inventory().get_stack(9).item_count, 1);
    assert!(fixture.payment().get_stack(0).is_empty());
    assert_eq!(
        fixture.effects(),
        (i32::from(pumpkin_data::effect::StatusEffect::SPEED.id), -1)
    );
    assert_eq!(fixture.sounds(), 1);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_beacon_current_payment_in_packet() {
    let mut fixture = Fixture::new(1).await;
    fixture.owner.take_packets();
    let action = payment_move(&fixture);
    let first = fixture.owner.player.inventory().get_stack(9);
    let mut remainder = first.clone();
    remainder.item_count = 1;
    let replenish = Action::Place {
        count: 1,
        source: slot(ContainerName::Inventory, 9, &remainder),
        destination: slot(ContainerName::BeaconPayment, 27, ItemStack::EMPTY),
    };
    send(
        &fixture.owner,
        vec![
            request(-1, vec![action]),
            request(-3, vec![selection(1, 0)]),
            request(-5, vec![replenish]),
            request(-7, vec![selection(1, 0)]),
        ],
    );
    let decoded = responses(&fixture.owner.take_packets());
    assert_eq!(
        decoded
            .iter()
            .map(|(id, status, _)| (*id, *status))
            .collect::<Vec<_>>(),
        [(-1, 0), (-3, 0), (-5, 0), (-7, 0)]
    );
    paid_response(&decoded[1].2);
    paid_response(&decoded[3].2);
    assert!(fixture.owner.player.inventory().get_stack(9).is_empty());
    assert!(fixture.payment().get_stack(0).is_empty());
    assert_eq!(fixture.sounds(), 2);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_prediction_authority_rejects_unretained_results() {
    for case in [
        "unknown",
        "fresh_uid",
        "failed_reuse",
        "wrong_address",
        "new_menu",
    ] {
        let mut fixture = Fixture::new(1).await;
        fixture.owner.take_packets();
        let action = payment_move(&fixture);
        send(&fixture.owner, vec![request(-1, vec![action])]);
        let mut actions = confirm(-1);
        match case {
            "unknown" => actions = confirm(-99),
            "fresh_uid" => {
                let fresh = fixture.payment().get_stack(0).copy_with_count(1);
                fixture.payment().set_stack(0, fresh);
            }
            "failed_reuse" => send(
                &fixture.owner,
                vec![request(-1, vec![Action::CraftNonImplemented])],
            ),
            "wrong_address" => {
                if let Action::Destroy { source, .. } = &mut actions[1] {
                    source.slot_id = 10;
                }
            }
            "new_menu" => {
                fixture.owner.player.close_handled_screen();
                fixture
                    .owner
                    .player
                    .inventory()
                    .set_stack_in_hand(pumpkin_util::Hand::Right, ItemStack::EMPTY.clone());
                fixture.open();
                let action = payment_move(&fixture);
                send(&fixture.owner, vec![request(-3, vec![action])]);
            }
            _ => unreachable!(),
        }
        fixture.owner.take_packets();
        let payment = fixture.payment().get_stack(0);
        let effects = fixture.effects();
        send(&fixture.owner, vec![request(-5, actions)]);
        let packets = fixture.owner.take_packets();
        assert_eq!(responses(&packets)[0].1, 1, "{case}");
        assert_eq!(fixture.payment().get_stack(0).uid, payment.uid, "{case}");
        assert!(fixture.payment().get_stack(0).are_equal(&payment), "{case}");
        assert_eq!(fixture.effects(), effects, "{case}");
        super::workstation_correction_test_support::assert_correction(
            &packets,
            &fixture.owner.player,
        );
        fixture.finish().await;
    }
}

fn deposit(fixture: &LoomFixture, id: i32, name: ContainerName, index: u8, stack: &ItemStack) {
    fixture
        .bedrock
        .player
        .inventory()
        .set_stack(usize::from(index), stack.clone());
    send(
        &fixture.bedrock,
        vec![request(
            id,
            vec![Action::Place {
                count: stack.item_count,
                source: slot(ContainerName::Inventory, index, stack),
                destination: slot(name, index, ItemStack::EMPTY),
            }],
        )],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_loom_unacknowledged_predictions() {
    let mut fixture = LoomFixture::empty().await;
    fixture.bedrock.take_packets();
    deposit(&fixture, -1, ContainerName::LoomInput, 9, &banner(2, 1));
    deposit(
        &fixture,
        -3,
        ContainerName::LoomDye,
        10,
        &ItemStack::new(2, &Item::RED_DYE),
    );
    let mut actions = fixture.craft_actions("bs", 1, false);
    for (index, reference) in [(2, -1), (3, -3), (4, -5)] {
        match &mut actions[index] {
            Action::Consume { source, .. } | Action::Take { source, .. } => {
                source.stack_id = reference;
            }
            _ => unreachable!(),
        }
    }
    send(&fixture.bedrock, vec![request(-5, actions)]);
    fixture.last_packets = fixture.bedrock.take_packets();
    let decoded = responses(&fixture.last_packets);
    assert_eq!(
        decoded
            .iter()
            .map(|(id, status, _)| (*id, *status))
            .collect::<Vec<_>>(),
        [(-1, 0), (-3, 0), (-5, 0)]
    );
    fixture.assert_updates(&decoded[2].2, false);
    assert_eq!(fixture.input(0).item_count, 1);
    assert_eq!(fixture.input(1).item_count, 1);
    assert_eq!(
        fixture.bedrock.player.inventory().get_stack(12).item_count,
        1
    );
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_loom_serialized_banner_components() {
    let mut fixture = LoomFixture::new(banner(2, 1), 2, None).await;
    let actions = fixture.craft_actions("bs", 1, false);
    fixture.submit(-10, actions, 0);
    let output = fixture.bedrock.player.inventory().get_stack(12);
    let mut seen = false;
    for packet in &fixture.last_packets {
        let Some(mut reader) = packet_payload(packet, CInventoryContent::PACKET_ID as u32) else {
            continue;
        };
        let window = VarUInt::read(&mut reader).unwrap().0;
        let slots = Vec::<NetworkItemStackDescriptor>::read(&mut reader).unwrap();
        FullContainerName::read(&mut reader).unwrap();
        NetworkItemStackDescriptor::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        if window != 0 {
            continue;
        }
        let actual = &slots[12];
        assert_eq!(actual.net_id, Some(output.uid));
        assert_eq!(actual.stack_size, 1);
        assert_banner_nbt(&actual.extra_data);
        seen = true;
    }
    assert!(seen, "crafted output inventory packet");
    // The wrapper is also used when the crafted banner becomes a dropped item.
    let mut bytes = Vec::new();
    ItemStackWrapper::from(&output).write(&mut bytes).unwrap();
    let mut reader = bytes.as_slice();
    let wrapper = ItemStackWrapper::read(&mut reader).unwrap();
    assert!(reader.is_empty());
    assert_tags(&wrapper.nbt_data);
    fixture.close().await;
}

fn assert_banner_nbt(extra_data: &[u8]) {
    let mut reader = std::io::Cursor::new(extra_data);
    assert_eq!(
        i16::read(&mut reader).unwrap(),
        -1,
        "banner has versioned NBT"
    );
    assert_eq!(u8::read(&mut reader).unwrap(), 1);
    let mut nbt_reader = pumpkin_nbt::deserializer::NbtReadHelperBedrock::new(&mut reader);
    let nbt = pumpkin_nbt::Nbt::read(&mut nbt_reader).unwrap();
    assert_tags(&nbt);
    assert_eq!(i32::read(&mut reader).unwrap(), 0);
    assert_eq!(i32::read(&mut reader).unwrap(), 0);
    assert_eq!(reader.position() as usize, extra_data.len());
}

fn assert_tags(nbt: &pumpkin_nbt::Nbt) {
    assert_eq!(
        nbt.get_compound("display")
            .unwrap()
            .get_string("Name")
            .unwrap(),
        "Keep my name"
    );
    let patterns = nbt.get_list("Patterns").unwrap();
    assert_eq!(patterns.len(), 2);
    for (layer, (pattern, color)) in patterns.iter().zip([("ts", 4), ("bs", 1)]) {
        let pumpkin_nbt::tag::NbtTag::Compound(layer) = layer else {
            unreachable!()
        };
        assert_eq!(layer.get_string("Pattern").unwrap(), pattern);
        assert_eq!(layer.get_int("Color").unwrap(), color);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_r3_loom_sound_once_per_tick() {
    let mut fixture = LoomFixture::new(banner(5, 1), 5, None).await;
    let world = fixture.world.world();
    world.level.set_game_time(1);
    let mut observer = TestPlayer::new(&world);
    observer.client().set_player(observer.player.clone());
    observer
        .player
        .get_entity()
        .set_pos(pumpkin_util::math::vector3::Vector3::new(8.5, 64.0, 9.5));
    world.players.store(Arc::new(vec![
        fixture.bedrock.player.clone(),
        observer.player.clone(),
    ]));
    observer.take_packets();
    for count in [2, 1] {
        let actions = fixture.craft_actions("bs", count, false);
        fixture.submit(-10, actions, 0);
    }
    assert_loom_sound(&observer.take_packets());
    world.level.set_game_time(world.level.get_game_time() + 1);
    let actions = fixture.craft_actions("bs", 1, false);
    fixture.submit(-10, actions, 0);
    assert_loom_sound(&observer.take_packets());
    world.level.set_game_time(world.level.get_game_time() + 1);
    let actions = fixture.craft_actions("unknown", 1, false);
    fixture.submit(-10, actions, 1);
    assert!(decode_sounds(&observer.take_packets()).is_empty());
    observer.client().close();
    fixture.close().await;
}

fn assert_loom_sound(packets: &[bytes::Bytes]) {
    let sounds = decode_sounds(packets)
        .into_iter()
        .filter(|sound| sound.sound_id == pumpkin_data::sound::Sound::UiLoomTakeResult as u16)
        .collect::<Vec<_>>();
    assert_eq!(sounds.len(), 1, "one loom sound per game tick");
    assert_eq!(
        sounds[0].category,
        pumpkin_data::sound::SoundCategory::Blocks as i32
    );
    assert_eq!(
        sounds[0].position,
        pumpkin_util::math::vector3::Vector3::new(68, 516, 68)
    );
    assert_eq!((sounds[0].volume, sounds[0].pitch), (1.0, 1.0));
}
