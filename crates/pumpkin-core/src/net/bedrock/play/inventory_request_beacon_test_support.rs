use super::*;

pub(super) struct Fixture {
    _directory: tempfile::TempDir,
    _server: Arc<Server>,
    pub(super) world: Arc<World>,
    pub(super) beacon: Arc<BeaconBlockEntity>,
    pub(super) owner: TestBedrockPlayer,
    pub(super) observer: TestPlayer,
}

impl Fixture {
    pub(super) async fn new(levels: i32) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        server.worlds.store(Arc::new(vec![world.clone()]));
        let position = BlockPos::new(8, 64, 8);
        combat_test_support::publish_empty_chunk(&world, position.chunk_position());
        for layer in 1..=levels {
            for x in 8 - layer..=8 + layer {
                for z in 8 - layer..=8 + layer {
                    world.set_block_state(
                        &BlockPos::new(x, 64 - layer, z),
                        Block::IRON_BLOCK.default_state.id,
                        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
                    );
                }
            }
        }
        world.set_block_state(
            &position,
            Block::BEACON.default_state.id,
            BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
        );
        let beacon = Arc::new(BeaconBlockEntity::new(position));
        world.add_block_entity(beacon.clone());
        for _ in 0..64 {
            beacon.tick(&world);
        }
        assert!(beacon.has_active_beam());
        assert_eq!(beacon.levels.load(Ordering::Relaxed), levels);
        let owner = TestBedrockPlayer::new(&world).await;
        owner
            .player
            .screen_handler_sync_handler
            .store_player(owner.player.clone());
        owner
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 10.5));
        let observer = TestPlayer::new(&world);
        observer.client().set_player(observer.player.clone());
        observer
            .player
            .get_entity()
            .set_pos(Vector3::new(8.5, 64.0, 10.5));
        world.players.store(Arc::new(vec![
            owner.player.clone(),
            observer.player.clone(),
        ]));
        let result = Self {
            _directory: directory,
            _server: server,
            world,
            beacon,
            owner,
            observer,
        };
        result.open();
        result
    }

    pub(super) fn screen(&self) -> SharedScreenHandler {
        self.owner
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone()
    }

    pub(super) fn payment(&self) -> Arc<dyn Inventory> {
        self.screen()
            .lock()
            .unwrap()
            .as_any()
            .downcast_ref::<BeaconScreenHandler>()
            .expect("normal Bedrock use must open a beacon")
            .inventory
            .clone()
    }

    pub(super) fn open(&self) {
        self.owner.client().handle_inventory_action(
            &self.owner.player,
            SInventoryTransaction {
                legacy_request_id: VarInt(0),
                legacy_set_item_slots: Vec::new(),
                has_value: false,
                actions: Vec::new(),
                transaction_type: VarUInt(2),
                transaction_data: TransactionData::UseItem(UseItemTransactionData {
                    action_type: VarInt(0),
                    trigger_type: 0,
                    block_position: self.beacon.position,
                    block_face: 1,
                    hot_bar_slot: VarInt(0),
                    hand: HandSlot::Mainhand,
                    item_in_hand: NetworkItemDescriptor::from(&ItemStack::EMPTY.clone()),
                    player_position: Vector3::new(8.5, 64.0, 10.5),
                    click_position: Vector3::new(0.5, 0.5, 0.5),
                    block_runtime_id: VarUInt(0),
                    client_prediction: 0,
                    client_cooldown_state: 0,
                }),
            },
        );
        self.payment();
    }

    pub(super) fn pay(&mut self) {
        let mut payment = ItemStack::new(2, &Item::EMERALD);
        payment.set_data_component(ContainerImpl {
            items: vec![(0, ItemStack::new(3, &Item::DIAMOND))],
        });
        self.owner.player.inventory.set_stack(9, payment.clone());
        let mut source = slot(ContainerName::Inventory, 9);
        source.stack_id = payment.uid.get();
        request(
            &mut self.owner,
            vec![Action::Place {
                count: 1,
                source,
                destination: slot(ContainerName::BeaconPayment, 27),
            }],
            0,
        );
        assert_eq!(
            self.owner.player.inventory.get_stack(9).item_count,
            1,
            "inventory setup control"
        );
        let inserted = self.payment().get_stack(0);
        assert_eq!(inserted.item_count, 1, "menu setup control");
        assert!(inserted.are_items_and_components_equal(&payment));
    }

    pub(super) fn selection_actions(&self, primary: i32, secondary: i32) -> Vec<Action> {
        // Dragonfly handleBeaconPayment: the client predicts one Destroy after
        // selection; BeaconInventoryTranslator maps payment to UI slot 27.
        let mut source = slot(ContainerName::BeaconPayment, 27);
        source.stack_id = self.payment().get_stack(0).uid.get();
        vec![
            selection(primary, secondary),
            Action::Destroy { count: 1, source },
        ]
    }

    pub(super) fn sounds(&mut self) -> usize {
        decode_sounds(&self.observer.take_packets())
            .into_iter()
            .filter(|sound| {
                sound.sound_id == pumpkin_data::sound::Sound::BlockBeaconPowerSelect as u16
            })
            .count()
    }

    pub(super) fn effects(&self) -> (i32, i32) {
        (
            self.beacon.primary_effect.load(Ordering::Relaxed),
            self.beacon.secondary_effect.load(Ordering::Relaxed),
        )
    }

    pub(super) async fn finish(self) {
        self.owner.close().await;
        self.observer.client().close();
        self.world.level.shutdown().await.unwrap();
    }
}

pub(super) fn slot(name: ContainerName, slot_id: u8) -> ItemStackRequestSlotInfo {
    ItemStackRequestSlotInfo {
        container_name: FullContainerName {
            container_name: name,
            dynamic_id: None,
        },
        slot_id,
        stack_id: 0,
    }
}

pub(super) fn selection(primary: i32, secondary: i32) -> Action {
    Action::BeaconPayment {
        primary_effect_id: VarInt(primary),
        secondary_effect_id: VarInt(secondary),
    }
}

pub(super) fn request(
    owner: &mut TestBedrockPlayer,
    actions: Vec<Action>,
    expected: u8,
) -> Vec<bytes::Bytes> {
    let (packets, result) = dispatch(owner, actions);
    assert_eq!(result, expected);
    packets
}

pub(super) fn dispatch(
    owner: &mut TestBedrockPlayer,
    actions: Vec<Action>,
) -> (Vec<bytes::Bytes>, u8) {
    owner.take_packets();
    owner.client().handle_item_stack_request(
        &owner.player,
        SItemStackRequest {
            requests: vec![ItemStackRequest {
                request_id: VarInt(-1),
                actions,
                filter_strings: Vec::new(),
                filter_cause: 0,
            }],
        },
    );
    let packets = owner.take_packets();
    let mut response = packets
        .iter()
        .find_map(|packet| packet_payload(packet, 148))
        .unwrap();
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    let result = u8::read(&mut response).unwrap();
    assert_eq!(VarInt::read(&mut response).unwrap().0, -1);
    if result != 0 {
        assert!(
            !bool::read(&mut response).unwrap(),
            "rejection has no success slots"
        );
        assert!(response.is_empty());
        super::super::workstation_correction_test_support::assert_correction(
            &packets,
            &owner.player,
        );
    }
    (packets, result)
}

pub(super) fn assert_payment_response(packets: &[bytes::Bytes]) {
    let mut response = packets
        .iter()
        .find_map(|packet| packet_payload(packet, 148))
        .unwrap();
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    assert_eq!(u8::read(&mut response).unwrap(), 0);
    assert_eq!(VarInt::read(&mut response).unwrap().0, -1);
    assert!(
        bool::read(&mut response).unwrap(),
        "payment count/UID update must be present"
    );
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    assert_eq!(
        FullContainerName::read(&mut response).unwrap(),
        FullContainerName {
            container_name: ContainerName::BeaconPayment,
            dynamic_id: None
        }
    );
    assert_eq!(VarUInt::read(&mut response).unwrap().0, 1);
    assert_eq!(u8::read(&mut response).unwrap(), 27);
    assert_eq!(u8::read(&mut response).unwrap(), 27);
    assert_eq!(u8::read(&mut response).unwrap(), 0);
    assert!(
        !bool::read(&mut response).unwrap(),
        "consumed payment has no network ID"
    );
    assert_eq!(String::read(&mut response).unwrap(), "");
    assert_eq!(String::read(&mut response).unwrap(), "");
    assert_eq!(VarInt::read(&mut response).unwrap().0, 0);
    assert!(response.is_empty());
}
