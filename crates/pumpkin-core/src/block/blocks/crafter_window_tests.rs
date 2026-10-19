use crate::{
    block::entities::{
        beacon::BeaconBlockEntity, chest::ChestBlockEntity, crafter::CrafterBlockEntity,
    },
    entity::{EntityBase, player::Player},
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{
    Block, data_component_impl::CustomNameImpl, item::Item, item_stack::ItemStack,
    packet::CURRENT_MC_VERSION, screen::WindowType,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    MultiVersionJavaPacket, Packet,
    bedrock::{
        BEDROCK_GAME_PACKET,
        client::container_open::CContainerOpen,
        network_item::NetworkItemDescriptor,
        packet_decoder::BedrockBatchDecoder,
        server::{
            container_close::SContainerClose,
            inventory_transaction::{
                HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
            },
        },
    },
    codec::{var_int::VarInt, var_uint::VarUInt},
    java::{client::play::COpenScreen, server::play::SUseItemOn},
    ser::NetworkReadExt,
    serial::PacketRead,
};
use pumpkin_util::{
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
    text::TextComponent,
    version::BedrockMinecraftVersion,
};
use std::{
    io::Cursor,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

const CRAFTER: BlockPos = BlockPos::new(8, 64, 8);
const CHEST: BlockPos = BlockPos::new(10, 64, 8);
const BEACON: BlockPos = BlockPos::new(12, 64, 8);

struct Fixture {
    _directory: tempfile::TempDir,
    server: Arc<Server>,
    world: Arc<World>,
    crafter: Arc<CrafterBlockEntity>,
    content: ItemStack,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
        for (position, block) in [
            (CRAFTER, &Block::CRAFTER),
            (CHEST, &Block::CHEST),
            (BEACON, &Block::BEACON),
        ] {
            chunk.set_block_absolute_y(
                position.0.x as usize,
                position.0.y,
                position.0.z as usize,
                block.default_state.id,
            );
        }
        world
            .level
            .loaded_chunks
            .insert(CRAFTER.chunk_position(), chunk);
        let crafter = Arc::new(CrafterBlockEntity::new(CRAFTER));
        let mut content = ItemStack::new(7, &Item::DIAMOND);
        content.set_data_component(CustomNameImpl {
            name: TextComponent::text("Crafter backing inventory"),
        });
        crafter.set_stack(0, content.clone());
        world.add_block_entity(crafter.clone());
        world.add_block_entity(Arc::new(ChestBlockEntity::new(CHEST)));
        world.add_block_entity(Arc::new(BeaconBlockEntity::new(BEACON)));
        Self {
            _directory: directory,
            server,
            world,
            crafter,
            content,
        }
    }

    async fn close(self) {
        self.world.level.shutdown().await.unwrap();
        self.server.shutdown().await;
    }
}

fn prepare_player(player: &Player) {
    player.permission_lvl.store(PermissionLvl::Four);
    player.get_entity().set_pos(Vector3::new(10.5, 64.0, 10.5));
    player.inventory.set_selected_slot(0);
    player.inventory.set_stack(0, ItemStack::EMPTY.clone());
}

fn use_bedrock_block(player: &TestBedrockPlayer, position: BlockPos) {
    player.client().handle_inventory_action(
        &player.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: false,
            actions: Vec::new(),
            transaction_type: VarUInt(2),
            transaction_data: TransactionData::UseItem(UseItemTransactionData {
                action_type: VarInt(0),
                trigger_type: 0,
                block_position: position,
                block_face: 3,
                hot_bar_slot: VarInt(0),
                hand: HandSlot::Mainhand,
                item_in_hand: NetworkItemDescriptor::default(),
                player_position: player.player.position().to_f32_lossy(),
                click_position: Vector3::new(0.5, 0.5, 1.0),
                block_runtime_id: VarUInt(0),
                client_prediction: 0,
                client_cooldown_state: 0,
            }),
        },
    );
}

fn bedrock_payloads(player: &mut TestBedrockPlayer, id: i32) -> Vec<Vec<u8>> {
    player
        .take_packets()
        .into_iter()
        .filter_map(|frame| {
            assert_eq!(frame[0], BEDROCK_GAME_PACKET);
            let packet = BedrockBatchDecoder::new()
                .get_game_packet(&mut Cursor::new(frame[1..].to_vec()))
                .unwrap();
            (packet.id == id).then(|| packet.payload.to_vec())
        })
        .collect()
}

fn one_open(player: &mut TestBedrockPlayer, position: BlockPos) -> (u8, u8) {
    let packets = bedrock_payloads(player, CContainerOpen::PACKET_ID);
    assert_eq!(packets.len(), 1, "actual block-use must emit ContainerOpen");
    let mut payload = packets[0].as_slice();
    let id = payload.get_u8().unwrap();
    let kind = payload.get_u8().unwrap();
    assert_eq!(BlockPos::read(&mut payload).unwrap(), position);
    (id, kind)
}

fn assert_crafter_owner(fixture: &Fixture, player: &Player) {
    let menu = player.current_screen_handler.lock().unwrap().clone();
    let menu = menu.lock().unwrap();
    assert_eq!(menu.window_type(), Some(WindowType::Crafter3x3));
    assert_eq!(menu.get_behaviour().slots.len(), 45);
    let slot = &menu.get_behaviour().slots[0];
    let expected: Arc<dyn Inventory> = fixture.crafter.clone();
    assert!(Arc::ptr_eq(&slot.get_inventory(), &expected));
    assert_eq!(slot.get_index(), 0);
    let actual = slot.get_cloned_stack();
    assert!(actual.are_equal(&fixture.content));
    assert_eq!(actual.uid, fixture.content.uid);
}

async fn crafter_windows() {
    let fixture = Fixture::new();
    let mut bedrock = TestBedrockPlayer::new(&fixture.world).await;
    bedrock
        .client()
        .version
        .store(BedrockMinecraftVersion::V_1_26_51);
    prepare_player(&bedrock.player);
    bedrock.take_packets();

    use_bedrock_block(&bedrock, CRAFTER);
    let first = one_open(&mut bedrock, CRAFTER);
    assert_crafter_owner(&fixture, &bedrock.player);

    bedrock.player.close_handled_screen();
    let closes = bedrock_payloads(&mut bedrock, SContainerClose::PACKET_ID);
    assert_eq!(closes.len(), 1);
    let close = SContainerClose::read(&mut closes[0].as_slice()).unwrap();
    assert_eq!(close.container_id, first.0);
    assert!(close.server_initiated_close);

    use_bedrock_block(&bedrock, CRAFTER);
    let reopened = one_open(&mut bedrock, CRAFTER);
    assert_ne!(first.0, reopened.0);
    assert_crafter_owner(&fixture, &bedrock.player);
    bedrock.client().handle_container_close(
        &bedrock.player,
        &SContainerClose {
            container_id: reopened.0,
            container_type: reopened.1,
            server_initiated_close: false,
        },
    );
    bedrock.take_packets();

    // The Wasm open_gui entry uses this direct opening API; there is no automatic crafter reopen.
    let factory = fixture
        .server
        .block_registry
        .get_screen_handler_factory(
            &Block::CRAFTER,
            &bedrock.player,
            &CRAFTER,
            &fixture.server,
            &fixture.world,
        )
        .unwrap();
    bedrock.player.increment_screen_handler_sync_id();
    let handler = factory
        .create_screen_handler(
            bedrock
                .player
                .screen_handler_sync_id
                .load(Ordering::Relaxed),
            &bedrock.player.inventory,
            bedrock.player.as_ref(),
        )
        .unwrap();
    bedrock
        .player
        .open_handled_screen_direct(handler, &factory.get_display_name());
    let direct = one_open(&mut bedrock, BlockPos::ZERO);
    assert_crafter_owner(&fixture, &bedrock.player);

    // Cloudburst ContainerType.CRAFTER is 36 for the fork's supported Bedrock protocol.
    assert_eq!(
        [first.1, close.container_type, reopened.1, direct.1],
        [36; 4],
        "crafter normal open, server close, block reopen and direct open must use dedicated type 36",
    );
    bedrock.player.close_handled_screen();
    bedrock.close().await;
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_bedrock_crafter_opens_dedicated_window() {
    tokio::time::timeout(Duration::from_secs(20), crafter_windows())
        .await
        .unwrap();
}

fn use_java_crafter(player: &mut TestPlayer, fixture: &Fixture) {
    player
        .client()
        .handle_use_item_on(
            &player.player,
            &SUseItemOn {
                hand: VarInt(0),
                position: CRAFTER,
                face: VarInt(3),
                cursor_pos: Vector3::new(0.5, 0.5, 1.0),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(1),
            },
            &fixture.server,
        )
        .unwrap();
    let opened: Vec<_> = player
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            if VarInt::decode(&mut data).unwrap().0 != COpenScreen::to_id(CURRENT_MC_VERSION) {
                return None;
            }
            let _sync_id = VarInt::decode(&mut data).unwrap();
            Some(VarInt::decode(&mut data).unwrap().0)
        })
        .collect();
    assert_eq!(opened, vec![WindowType::Crafter3x3 as i32]);
    assert_crafter_owner(fixture, &player.player);
}

async fn opening_controls() {
    let fixture = Fixture::new();
    let mut java = TestPlayer::new(&fixture.world);
    prepare_player(&java.player);
    java.take_packets();
    use_java_crafter(&mut java, &fixture);
    java.player.close_handled_screen();
    use_java_crafter(&mut java, &fixture);
    java.player.close_handled_screen();

    let mut bedrock = TestBedrockPlayer::new(&fixture.world).await;
    bedrock
        .client()
        .version
        .store(BedrockMinecraftVersion::V_1_26_51);
    prepare_player(&bedrock.player);
    bedrock.take_packets();
    for (position, expected) in [(CHEST, 0), (BEACON, 13)] {
        use_bedrock_block(&bedrock, position);
        let (_, actual) = one_open(&mut bedrock, position);
        assert_eq!(actual, expected, "ordinary container mapping changed");
        bedrock.player.close_handled_screen();
        bedrock.take_packets();
    }
    bedrock.close().await;
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_bedrock_crafter_opening_controls() {
    tokio::time::timeout(Duration::from_secs(20), opening_controls())
        .await
        .unwrap();
}
