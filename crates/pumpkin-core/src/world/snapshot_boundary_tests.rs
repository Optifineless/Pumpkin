#[cfg(debug_assertions)]
use crate::{
    block::entities::capture::{
        capture_legacy,
        tests::{CaptureAction, TestCapture, rejected},
    },
    net::java::combat_test_support::TestPlayer,
    plugin::world::world_save::WorldSaveEvent,
    server::combat_test_support::{server, world},
};
use crate::{
    block::entities::{
        BlockEntity, chest::ChestBlockEntity, comparator::ComparatorBlockEntity,
        map::MapBlockEntity,
    },
    server::combat_test_support::publish_empty_chunk,
};
#[cfg(debug_assertions)]
use futures::FutureExt as _;
#[cfg(debug_assertions)]
use pumpkin_data::{Block, data_component::DataComponent, data_component_impl::DataComponentImpl};
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::position::BlockPos;
#[cfg(debug_assertions)]
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(debug_assertions)]
async fn snapshot_boundary_case(case: usize) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut player = TestPlayer::new(&world);
    player.take_packets();
    let pos = BlockPos::new(8, 64, 8);
    let actions: Vec<(&str, CaptureAction)> = vec![
        ("Level::read_chunk_sync", {
            let world = world.clone();
            Arc::new(move |_| {
                world.level.read_chunk_sync(&pos.chunk_position(), |_| ());
            })
        }),
        ("World::get_block_entity", {
            let world = world.clone();
            Arc::new(move |_| {
                world.get_block_entity(&pos);
            })
        }),
        ("World::set_block_state", {
            let world = world.clone();
            Arc::new(move |_| {
                world.set_block_state(&pos, Block::AIR.default_state.id, BlockFlags::empty());
            })
        }),
        ("ClientPlatform::try_enqueue_packet", {
            let client = player.player.client.clone();
            Arc::new(move |_| {
                client.try_enqueue_packet(bytes::Bytes::new());
            })
        }),
        ("PluginManager::fire_blocking", {
            let server = server.clone();
            Arc::new(move |_| {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut WorldSaveEvent::new("snapshot-test".into()));
            })
        }),
        (
            "GuardEffects::defer",
            Arc::new(|_| {
                let _ = super::guard_effects::defer(|| {});
            }),
        ),
        (
            "OwnerEffects::capture",
            Arc::new(|_| {
                super::guard_effects::OwnerEffects::default().capture(|| {});
            }),
        ),
        ("World::break_straw_bed_half", {
            let world = world.clone();
            Arc::new(move |_| {
                let _ = world.break_straw_bed_half(
                    &pos,
                    Block::STRAW_BED.default_state.id,
                    BlockFlags::empty(),
                    super::neighbor_updater::UPDATE_LIMIT,
                );
            })
        }),
    ];
    for (boundary, action) in actions.into_iter().skip(case).take(1) {
        // Install a native BE and enter through the ordinary world save, without handler hooks.
        world.replace_live_block_entity(pos, Arc::new(TestCapture(action)));
        rejected(
            || world.save_block_entities(pos.chunk_position(), false),
            &format!("boundary {boundary}:"),
        );
        // Both scope and transitive markers must be gone after unwinding.
        assert!(world.get_block_entity(&pos).is_some());
        assert!(
            world
                .level
                .read_chunk_sync(&pos.chunk_position(), |_| ())
                .is_none()
        );
        assert!(player.take_packets().is_empty());
    }
    world.remove_block_entity(&pos);
    world.level.shutdown().await.unwrap();
}

#[cfg(debug_assertions)]
macro_rules! boundary_test {
    ($name:ident, $case:literal) => {
        #[tokio::test]
        async fn $name() {
            snapshot_boundary_case($case).await;
        }
    };
}
#[cfg(debug_assertions)]
mod reentry {
    use super::*;
    boundary_test!(snapshot_rejects_chunk_read, 0);
    boundary_test!(snapshot_rejects_live_lookup, 1);
    boundary_test!(snapshot_rejects_world_mutation, 2);
    boundary_test!(snapshot_rejects_packet, 3);
    boundary_test!(snapshot_rejects_event, 4);
    boundary_test!(snapshot_rejects_deferred_effect, 5);
    boundary_test!(snapshot_rejects_effect_capture, 6);
    boundary_test!(snapshot_rejects_straw_cleanup, 7);
}

#[cfg(debug_assertions)]
struct ReentrantComponent(Arc<super::World>, bool);
#[cfg(debug_assertions)]
impl DataComponentImpl for ReentrantComponent {
    fn write_data(&self) -> NbtTag {
        self.0.get_block_entity(&BlockPos::new(8, 64, 8));
        NbtTag::End
    }
    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        if self.1 {
            self.0.get_block_entity(&BlockPos::new(8, 64, 8));
        }
        Box::new(Self(self.0.clone(), self.1))
    }
    fn equal(&self, _: &dyn DataComponentImpl) -> bool {
        false
    }
    fn get_enum() -> DataComponent {
        DataComponent::CustomData
    }
    fn get_self_enum(&self) -> DataComponent {
        Self::get_enum()
    }
    fn to_dyn(self) -> Box<dyn DataComponentImpl> {
        Box::new(self)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(debug_assertions)]
async fn transitive_case(clone: bool) {
    let fixture = super::spawn_test_support::Fixture::new();
    let component: Box<dyn DataComponentImpl> =
        Box::new(ReentrantComponent(fixture.world.clone(), clone));
    let entity = TestCapture(Arc::new(move |nbt| {
        if clone {
            let _ = component.clone();
        } else {
            let mut item = ItemStack::new(1, &Item::DIAMOND);
            item.patch
                .push((DataComponent::CustomData, Some(component.clone())));
            item.write_item_stack(nbt);
        }
    }));
    let expected = if clone {
        "DataComponentImpl::clone_dyn"
    } else {
        "DataComponentImpl::write_data"
    };
    rejected(
        || {
            let _ = capture_legacy(&entity);
        },
        expected,
    );
    assert!(
        fixture
            .world
            .get_block_entity(&BlockPos::new(8, 64, 8))
            .is_none()
    );
    fixture.finish().await;
}

#[tokio::test]
#[cfg(debug_assertions)]
async fn snapshot_rejects_transitive_clone() {
    transitive_case(true).await;
}
#[tokio::test]
#[cfg(debug_assertions)]
async fn snapshot_rejects_transitive_encode() {
    transitive_case(false).await;
}

#[tokio::test]
async fn snapshot_normal_and_final_save_preserve_legacy_fixture_nbt_bytes() {
    let fixture = super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = BlockPos::new(8, 64, 8);
    publish_empty_chunk(world, pos.chunk_position());
    let chunk = world
        .level
        .loaded_chunks
        .get(&pos.chunk_position())
        .unwrap()
        .clone();
    let chest = ChestBlockEntity::new(pos);
    chest.set_stack(0, ItemStack::new(17, &Item::DIAMOND));
    let fixtures: Vec<Arc<dyn BlockEntity>> = vec![
        Arc::new(chest),
        Arc::new(ComparatorBlockEntity::new(pos)),
        Arc::new(MapBlockEntity::new(pos, 42)),
    ];
    let mut custom = NbtCompound::new();
    custom.put_string("fixture", "legacy custom data".into());
    world.custom_block_entity_data.insert(pos, custom.clone());
    for entity in fixtures {
        let mut expected = NbtCompound::new();
        entity.write_internal(&mut expected);
        expected.put_compound("PumpkinCustomData", custom.clone());
        let bytes = canonical_bytes(expected);
        world.replace_live_block_entity(pos, entity);
        world.save_block_entities(pos.chunk_position(), false);
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&pos)
            .unwrap()
            .clone();
        assert_eq!(canonical_bytes(saved), bytes);
        world.unload_block_entities(pos.chunk_position());
        let saved = chunk
            .pending_block_entities
            .lock()
            .unwrap()
            .get(&pos)
            .unwrap()
            .clone();
        assert_eq!(canonical_bytes(saved), bytes);
        assert!(!world.block_entities.contains_key(&pos.chunk_position()));
    }
    fixture.finish().await;
}

#[tokio::test]
async fn shutdown_waits_for_admitted_block_entity_mutations_before_closing_admission() {
    let fixture = super::spawn_test_support::Fixture::new();
    let world = &fixture.world;
    let pos = BlockPos::new(8, 64, 8);
    let admitted = world
        .block_entity_mutation_permit(pos)
        .expect("fixture world should admit the first mutation");

    assert!(!world.try_stop_block_entity_mutations());
    let nested = world
        .block_entity_mutation_permit(pos)
        .expect("an admitted callback must finish its nested block-entity registration");
    drop(nested);
    drop(admitted);

    assert!(world.try_stop_block_entity_mutations());
    assert!(world.block_entity_mutation_permit(pos).is_none());
    fixture.finish().await;
}

// NbtCompound uses randomized HashMap order. Compare known fixture bytes with
// compound keys sorted; primitive payloads still use the production Java codec.
fn canonical_bytes(nbt: NbtCompound) -> Vec<u8> {
    use pumpkin_nbt::serializer::{NbtWriteHelper, NbtWriteHelperJava};
    fn payload(tag: NbtTag, w: &mut NbtWriteHelperJava<&mut Vec<u8>>) {
        match tag {
            NbtTag::Compound(nbt) => {
                let mut entries: Vec<_> = nbt.child_tags.into_iter().collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                for (name, tag) in entries {
                    w.write_u8(tag.get_type_id()).unwrap();
                    w.write_string(&name).unwrap();
                    payload(tag, w);
                }
                w.write_u8(pumpkin_nbt::END_ID).unwrap();
            }
            NbtTag::List(list) => {
                w.write_u8(
                    list.first()
                        .map_or(pumpkin_nbt::END_ID, NbtTag::get_type_id),
                )
                .unwrap();
                w.write_i32(list.len().try_into().unwrap()).unwrap();
                for tag in list {
                    payload(tag, w);
                }
            }
            tag => tag.serialize_data(w).unwrap(),
        }
    }
    let mut bytes = Vec::new();
    let mut writer = NbtWriteHelperJava::new(&mut bytes);
    writer.write_u8(pumpkin_nbt::COMPOUND_ID).unwrap();
    payload(NbtTag::Compound(nbt), &mut writer);
    bytes
}

#[cfg(debug_assertions)]
struct ProbeJavaPacket(Arc<AtomicBool>);

#[cfg(debug_assertions)]
impl pumpkin_protocol::MultiVersionJavaPacket for ProbeJavaPacket {
    fn to_id(_: pumpkin_util::version::JavaMinecraftVersion) -> i32 {
        0
    }
}

#[cfg(debug_assertions)]
impl pumpkin_protocol::ClientPacket for ProbeJavaPacket {
    fn write_packet_data(
        &self,
        _: impl std::io::Write,
        _: &pumpkin_util::version::JavaMinecraftVersion,
    ) -> Result<(), pumpkin_protocol::ser::WritingError> {
        self.0.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(debug_assertions)]
struct ProbeBedrockPacket;

#[cfg(debug_assertions)]
impl pumpkin_protocol::Packet for ProbeBedrockPacket {
    const PACKET_ID: i32 = 0;
}

#[cfg(debug_assertions)]
impl pumpkin_protocol::BClientPacket for ProbeBedrockPacket {
    fn write_packet(&self, _: impl std::io::Write) -> Result<(), std::io::Error> {
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn rejected_capture_boundary(expected: &str, operation: impl Fn() + Send + Sync + 'static) {
    let capture = TestCapture(Arc::new(move |_| operation()));
    rejected(
        || {
            let _ = capture_legacy(&capture);
        },
        expected,
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_editioned_packet_encoding_before_writer() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let client = player.player.client.clone();
    let encoded = Arc::new(AtomicBool::new(false));
    let packet = ProbeJavaPacket(encoded.clone());
    rejected_capture_boundary("ClientPlatform::try_enqueue_packet_editioned", move || {
        client.try_enqueue_packet_editioned(&packet, &ProbeBedrockPacket);
    });
    assert!(!encoded.load(Ordering::Relaxed));
    world.level.shutdown().await.unwrap();
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_editioned_enqueue_before_writer() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let client = player.player.client.clone();
    let encoded = Arc::new(AtomicBool::new(false));
    let packet = ProbeJavaPacket(encoded.clone());
    // Poll only the synchronous prefix, where the guard must run before serialization or await.
    rejected_capture_boundary("ClientPlatform::enqueue_packet_editioned", move || {
        let _ = client
            .enqueue_packet_editioned(&packet, &ProbeBedrockPacket)
            .now_or_never();
    });
    assert!(!encoded.load(Ordering::Relaxed));
    world.level.shutdown().await.unwrap();
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_editioned_send_before_writer() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let client = player.player.client.clone();
    let encoded = Arc::new(AtomicBool::new(false));
    let packet = ProbeJavaPacket(encoded.clone());
    rejected_capture_boundary("ClientPlatform::send_packet_now_editioned", move || {
        let _ = client
            .send_packet_now_editioned(&packet, &ProbeBedrockPacket)
            .now_or_never();
    });
    assert!(!encoded.load(Ordering::Relaxed));
    world.level.shutdown().await.unwrap();
}

#[cfg(debug_assertions)]
async fn rejects_empty_recipient_broadcast(
    expected: &'static str,
    broadcast: impl Fn(&Arc<super::World>, &ProbeJavaPacket, &ProbeBedrockPacket)
    + Send
    + Sync
    + 'static,
) {
    let fixture = super::spawn_test_support::Fixture::new();
    let world = fixture.world.clone();
    let encoded = Arc::new(AtomicBool::new(false));
    let packet = ProbeJavaPacket(encoded.clone());
    rejected_capture_boundary(expected, move || {
        broadcast(&world, &packet, &ProbeBedrockPacket);
    });
    assert!(!encoded.load(Ordering::Relaxed));
    fixture.finish().await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_except_broadcast_before_recipient_filtering() {
    rejects_empty_recipient_broadcast("World::broadcast_packet_except", |world, packet, _| {
        world.broadcast_packet_except(&[], packet);
    })
    .await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_editioned_except_broadcast_before_recipient_filtering() {
    rejects_empty_recipient_broadcast(
        "World::broadcast_packet_except_editioned",
        |world, packet, bedrock| {
            world.broadcast_packet_except_editioned(&[], packet, bedrock);
        },
    )
    .await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_chunk_except_broadcast_before_recipient_filtering() {
    rejects_empty_recipient_broadcast("World::broadcast_to_chunk_except", |world, packet, _| {
        world.broadcast_to_chunk_except(
            pumpkin_util::math::vector2::Vector2::new(0, 0),
            &[],
            packet,
        );
    })
    .await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_editioned_chunk_except_broadcast_before_recipient_filtering() {
    rejects_empty_recipient_broadcast(
        "World::broadcast_to_chunk_except_editioned",
        |world, packet, bedrock| {
            world.broadcast_to_chunk_except_editioned(
                pumpkin_util::math::vector2::Vector2::new(0, 0),
                &[],
                packet,
                bedrock,
            );
        },
    )
    .await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn snapshot_rejects_chat_filter_boundary_before_callback() {
    let fixture = super::spawn_test_support::Fixture::new();
    let world = fixture.world.clone();
    let called = Arc::new(AtomicBool::new(false));
    let filter_called = called.clone();
    let message = crate::net::chat::PlayerChatMessage::system("snapshot filter".into());
    rejected_capture_boundary("World::broadcast_chat_message", move || {
        let filter_called = filter_called.clone();
        world.broadcast_chat_message(
            &message,
            move |_| {
                filter_called.store(true, Ordering::Relaxed);
                false
            },
            None,
            0.into(),
            &pumpkin_util::text::TextComponent::empty(),
            None,
        );
    });
    assert!(!called.load(Ordering::Relaxed));
    fixture.finish().await;
}
