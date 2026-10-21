use super::*;
use crate::block::{
    BlockBehaviour, GetScreenHandlerFactoryArgs, blocks::crafting_table::CraftingTableBlock,
};
use bytes::Bytes;
use pumpkin_data::{data_component::DataComponent, packet::CURRENT_MC_VERSION};
use pumpkin_inventory::screen_handler::SharedScreenHandler;
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    codec::item_stack_seralizer::{ItemStackSerializer, OptionalItemStackHash},
    java::{
        client::play::CSetContainerSlot,
        server::play::{SClickSlot, SlotActionType},
    },
    packet::MultiVersionJavaPacket,
    ser::NetworkWriteExt,
};

// Independently encoded HashOps CRC32C for four count-one templates, with custom
// names craft-back/left/right/front and angler/archer/blade/brewer pottery sherds.
const PATTERNED_POT_HASH: u32 = 0x699d_c7b8;

fn pot_claim(checksum: u32) -> OptionalItemStackHash {
    let mut wire = vec![1];
    wire.write_var_int(&VarInt(i32::from(Item::DECORATED_POT.id)))
        .unwrap();
    wire.extend_from_slice(&[1, 1]);
    wire.write_var_int(&VarInt(i32::from(DataComponent::PotDecorations.to_id())))
        .unwrap();
    wire.extend_from_slice(&checksum.to_be_bytes());
    wire.push(0);
    OptionalItemStackHash::read(&mut wire.as_slice()).unwrap()
}

fn open_crafting_table(fixture: &Fixture) -> (u8, SharedScreenHandler) {
    let position = BlockPos::new(8, 64, 8);
    fixture.set_block(position, &Block::CRAFTING_TABLE);
    fixture.approach(position);
    fixture
        .player
        .player
        .screen_handler_sync_handler
        .store_player(fixture.player.player.clone());
    let factory = CraftingTableBlock
        .get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: &fixture.server,
            world: &fixture.world,
            block: &Block::CRAFTING_TABLE,
            position: &position,
            player: &fixture.player.player,
        })
        .unwrap();
    let menu_id = fixture
        .player
        .player
        .open_handled_screen(factory.as_ref(), Some(position))
        .unwrap();
    let menu = fixture
        .player
        .player
        .current_screen_handler
        .lock()
        .unwrap()
        .clone();
    (menu_id, menu)
}

fn click(
    fixture: &Fixture,
    menu_id: u8,
    revision: u32,
    slot: i16,
    changed: Vec<(i16, OptionalItemStackHash)>,
    cursor: OptionalItemStackHash,
) {
    let mut payload = Vec::new();
    SClickSlot {
        sync_id: VarInt(i32::from(menu_id)),
        revision: VarInt(revision as i32),
        slot,
        button: SClickSlot::BUTTON_LEFT,
        mode: SlotActionType::Pickup,
        length_of_array: VarInt(changed.len() as i32),
        array_of_changed_slots: changed,
        carried_item: cursor,
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
                id: SClickSlot::to_id(CURRENT_MC_VERSION),
                payload: Bytes::from(payload),
            },
        )
        .unwrap();
}

async fn craft_and_move_pot(corrupt_claim: bool) -> bool {
    let mut fixture = Fixture::new();
    let (menu_id, menu) = open_crafting_table(&fixture);
    let mut crafting = menu.lock().unwrap();
    for ((slot, item), name) in [2, 4, 6, 8]
        .into_iter()
        .zip([
            &Item::ANGLER_POTTERY_SHERD,
            &Item::ARCHER_POTTERY_SHERD,
            &Item::BLADE_POTTERY_SHERD,
            &Item::BREWER_POTTERY_SHERD,
        ])
        .zip(["craft-back", "craft-left", "craft-right", "craft-front"])
    {
        let mut stack = ItemStack::new(1, item);
        stack.set_custom_name(name.to_owned());
        crafting.get_behaviour().slots[slot].set_stack(stack);
    }
    crafting.send_content_updates();
    drop(crafting);
    let revision = menu.lock().unwrap().get_behaviour().revision.load(Relaxed);
    fixture.player.take_packets();
    click(
        &fixture,
        menu_id,
        revision,
        0,
        [0, 2, 4, 6, 8]
            .map(|slot| (slot, OptionalItemStackHash(None)))
            .to_vec(),
        pot_claim(PATTERNED_POT_HASH),
    );
    let crafted = menu
        .lock()
        .unwrap()
        .get_behaviour()
        .cursor_stack
        .lock()
        .unwrap()
        .clone();
    assert_eq!(crafted.item, &Item::DECORATED_POT);
    assert_eq!(crafted.item_count, 1);
    let revision = menu.lock().unwrap().get_behaviour().revision.load(Relaxed);
    fixture.player.take_packets();
    click(
        &fixture,
        menu_id,
        revision,
        10,
        vec![(
            10,
            pot_claim(if corrupt_claim { 0 } else { PATTERNED_POT_HASH }),
        )],
        OptionalItemStackHash(None),
    );
    let (revision, moved) = {
        let menu = menu.lock().unwrap();
        let behaviour = menu.get_behaviour();
        assert!(behaviour.cursor_stack.lock().unwrap().is_empty());
        for slot in 0..10 {
            assert!(behaviour.slots[slot].get_stack().is_empty());
        }
        let moved = behaviour.slots[10].get_cloned_stack();
        assert!(moved.are_equal(&crafted));
        assert_eq!(
            behaviour.slots[10..]
                .iter()
                .map(|slot| u32::from(slot.get_stack().item_count))
                .sum::<u32>(),
            1
        );
        (behaviour.revision.load(Relaxed), moved)
    };
    let correction = fixture
        .player
        .client()
        .serialize_packet(&CSetContainerSlot::new(
            menu_id as i8,
            revision as i32,
            10,
            &ItemStackSerializer::from(moved),
        ))
        .unwrap();
    let corrected = fixture.player.take_packets().contains(&correction);
    fixture.shutdown().await;
    corrected
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_patterned_pot_hash_matches_vanilla_click() {
    assert!(!craft_and_move_pot(false).await);
    assert!(craft_and_move_pot(true).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_patterned_pot_click_conserves_authoritative_items() {
    // Prediction corrections do not reject the authoritative inventory action.
    let _corrected = craft_and_move_pot(false).await;
}
