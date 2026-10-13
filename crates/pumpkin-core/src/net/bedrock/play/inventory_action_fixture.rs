use super::*;

pub(super) struct HandUseFixture {
    pub(super) world: DeathTestWorld,
    pub(super) owner: TestBedrockPlayer,
    pub(super) java_observer: TestPlayer,
    pub(super) bedrock_observer: TestBedrockPlayer,
    pub(super) hands_before: Option<(u8, ItemStack, ItemStack)>,
}

impl HandUseFixture {
    pub(super) async fn new() -> Self {
        let fixture = DeathTestWorld::new().await;
        let world = fixture.world();
        publish(&world, proto(&Biome::PLAINS, &Block::AIR));
        let owner = TestBedrockPlayer::new(&world).await;
        owner
            .player
            .permission_lvl
            .store(pumpkin_util::permission::PermissionLvl::Four);
        let java_observer = TestPlayer::new(&world);
        let bedrock_observer = TestBedrockPlayer::new(&world).await;
        world.players.store(Arc::new(vec![
            owner.player.clone(),
            java_observer.player.clone(),
            bedrock_observer.player.clone(),
        ]));
        let tracked = world
            .entity_tracker
            .get_tracked_entity(owner.player.entity_id());
        assert!(tracked.is_some());
        if let Some(tracked) = tracked {
            for observer in [&java_observer.player, &bedrock_observer.player] {
                let chunk = owner.player.get_entity().chunk_pos.load();
                observer.watched_section.store(
                    pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
                        chunk,
                        std::num::NonZeroU8::new(2).unwrap(),
                    ),
                );
                observer
                    .chunk_sender
                    .lock()
                    .unwrap()
                    .mark_sent_out_of_band(chunk);
                tracked.update_player(observer, &world);
            }
        }
        Self {
            world: fixture,
            owner,
            java_observer,
            bedrock_observer,
            hands_before: None,
        }
    }

    pub(super) fn drain_packets(&mut self) {
        self.owner.player.tick(&self.world.server);
        self.hands_before = Some((
            self.owner.player.inventory().get_selected_slot(),
            self.owner.player.inventory().held_item(),
            self.owner.player.inventory().off_hand_item(),
        ));
        self.owner.take_packets();
        self.java_observer.take_packets();
        self.bedrock_observer.take_packets();
    }

    pub(super) fn hand_changed(&self, hand: Hand) -> bool {
        self.hands_before
            .as_ref()
            .is_none_or(|(selected, main, off)| {
                let inventory = self.owner.player.inventory();
                match hand {
                    Hand::Right => {
                        *selected != inventory.get_selected_slot()
                            || !main.are_equal(&inventory.held_item())
                    }
                    Hand::Left => !off.are_equal(&inventory.off_hand_item()),
                }
            })
    }

    pub(super) async fn close(self) {
        self.owner.close().await;
        self.bedrock_observer.close().await;
        self.world.server.shutdown().await;
    }

    pub(super) async fn verify_and_close(
        mut self,
        verify: impl FnOnce(&mut Self) -> TestResult,
    ) -> TestResult {
        self.owner.player.tick(&self.world.server);
        self.owner.player.tick(&self.world.server);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify(&mut self)));
        self.close().await;
        match result {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

pub(super) fn assert_owner_packet(
    fixture: &HandUseFixture,
    owner_packets: &[bytes::Bytes],
    hand: Hand,
    expected: &ItemStack,
) -> TestResult {
    let (window, slot, _) = hand_address(hand);
    assert_owner_inventory_slot(
        owner_packets,
        window,
        slot,
        expected,
        !fixture.hand_changed(hand),
    )
}

pub(super) fn assert_owner_inventory_slot(
    packets: &[bytes::Bytes],
    window: i32,
    slot: u32,
    expected: &ItemStack,
    allow_unchanged: bool,
) -> TestResult {
    use crate::net::bedrock::play::inventory_exploit_tests::packet_payload;
    use pumpkin_protocol::serial::PacketRead;
    let mut latest = None;
    let mut full = 0;
    for packet in packets {
        if let Some(mut bytes) = packet_payload(packet, 49) {
            if VarUInt::read(&mut bytes)?.0 != window as u32 {
                continue;
            }
            let count = VarUInt::read(&mut bytes)?.0;
            for index in 0..count {
                let stack = NetworkItemStackDescriptor::read(&mut bytes)?;
                if index == slot {
                    latest = Some(stack);
                    full += 1;
                }
            }
        } else if let Some(mut bytes) = packet_payload(packet, 50) {
            if VarUInt::read(&mut bytes)?.0 != window as u32 || VarUInt::read(&mut bytes)?.0 != slot
            {
                continue;
            }
            let _name = Option::<FullContainerName>::read(&mut bytes)?;
            let _storage = Option::<NetworkItemStackDescriptor>::read(&mut bytes)?;
            latest = Some(NetworkItemStackDescriptor::read(&mut bytes)?);
        }
    }
    // Owner prediction is replayed in order; a send callback can supersede the first correction.
    assert!(full <= 1, "only one final full inventory correction");
    let Some(actual) = latest else {
        assert!(allow_unchanged, "owner authoritative hand correction");
        return Ok(());
    };
    let expected = NetworkItemStackDescriptor::from(expected);
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.stack_size, expected.stack_size);
    assert_eq!(actual.aux_value.0, expected.aux_value.0);
    assert_eq!(actual.net_id, expected.net_id);
    assert_eq!(actual.extra_data, expected.extra_data);
    Ok(())
}
