#![expect(
    clippy::unwrap_used,
    reason = "Real lifecycle fixtures and bounded callback completion"
)]

use super::{
    DEADLINE, Fixture, ItemFrameEntity, dig, import, loaded, markers, persisted, place, stack,
};
use crate::{
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
    server::Server,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{CustomDataImpl, DataComponentImpl},
    item::Item,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::server::play::SAttack;
use pumpkin_util::math::vector2::Vector2;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_unload_reload_old_handle_cannot_withdraw_new_incarnation() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    let uuid = frame.get_entity().entity_uuid;
    let runtime_id = frame.get_entity().entity_id;
    let old = Arc::downgrade(&frame);
    let pos = Vector2::new(0, 0);
    let storage = tokio::time::timeout(DEADLINE, fixture.world.level.get_entity_chunk(pos))
        .await
        .unwrap()
        .unwrap();
    fixture.world.make_chunk_entities_live(&storage, None);
    tokio::time::timeout(DEADLINE, fixture.world.remove_entities_in_chunks([pos]))
        .await
        .unwrap();
    fixture.tick(1);
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    markers(&map, false);
    tokio::time::timeout(
        DEADLINE,
        fixture.world.level.mark_chunks_as_newly_watched(&[pos]),
    )
    .await
    .unwrap();
    let next = tokio::time::timeout(DEADLINE, fixture.world.level.get_entity_chunk(pos))
        .await
        .unwrap()
        .unwrap();
    fixture.world.make_chunk_entities_live(&next, None);
    let replacement = fixture.world.get_entity_by_uuid(uuid).unwrap();
    assert_ne!(replacement.get_entity().entity_id, runtime_id);
    assert_eq!(replacement.get_entity().entity_uuid, uuid);
    drop(frame);
    assert!(
        old.upgrade().is_none(),
        "old incarnation must actually be destroyed after reload"
    );
    dig(&fixture);
    fixture.tick(101);
    assert!(replacement.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 1);
    markers(&map, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}

#[derive(Clone, Copy)]
enum CloneAction {
    Replace,
    Creative,
    DisableDrops,
    EnableDrops,
}

struct ReplaceOnClone {
    action: CloneAction,
    after_clear: bool,
    player: Weak<Player>,
    data: CustomDataImpl,
    target: Weak<dyn EntityBase>,
    armed: Arc<AtomicBool>,
}

impl DataComponentImpl for ReplaceOnClone {
    fn write_data(&self) -> NbtTag {
        self.data.write_data()
    }
    fn clone_dyn(&self) -> Box<dyn DataComponentImpl> {
        if let Some(frame) = self.target.upgrade() {
            let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
            if (!self.after_clear || concrete.get_analog_output() == 0)
                && self.armed.swap(false, Ordering::SeqCst)
            {
                match self.action {
                    CloneAction::Replace => concrete.set_item(stack(1), false),
                    CloneAction::Creative => {
                        assert!(
                            self.player
                                .upgrade()
                                .unwrap()
                                .set_gamemode(pumpkin_util::GameMode::Creative)
                        );
                    }
                    CloneAction::DisableDrops | CloneAction::EnableDrops => {
                        let world = frame.get_entity().world.load_full();
                        world.level_info.rcu(|info| {
                            let mut info = (**info).clone();
                            info.game_rules.entity_drops =
                                matches!(self.action, CloneAction::EnableDrops);
                            info
                        });
                    }
                }
            }
        }
        Box::new(Self {
            action: self.action,
            after_clear: self.after_clear,
            player: self.player.clone(),
            data: self.data.clone(),
            target: self.target.clone(),
            armed: self.armed.clone(),
        })
    }
    fn equal(&self, other: &dyn DataComponentImpl) -> bool {
        self.data.equal(other)
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
        &self.data
    }
    fn as_mut_any(&mut self) -> &mut dyn std::any::Any {
        &mut self.data
    }
}

fn attack(player: &Arc<Player>, server: &Arc<Server>, frame: &dyn EntityBase) {
    assert!(
        matches!(player.client.as_ref(), ClientPlatform::Java(_)),
        "Java fixture"
    );
    if let ClientPlatform::Java(client) = player.client.as_ref() {
        player.last_attacked_ticks.store(100, Ordering::Relaxed);
        client.handle_attack(
            player,
            &SAttack {
                entity_id: frame.get_entity().entity_id.into(),
            },
            server,
        );
    }
}

fn bounded_attack(fixture: &Fixture, frame: &Arc<dyn EntityBase>) {
    let player = fixture.player.player.clone();
    let server = fixture.server.clone();
    let target = frame.clone();
    let runtime = tokio::runtime::Handle::current();
    let (send, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        attack(&player, &server, target.as_ref());
        send.send(()).unwrap();
    });
    done.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    worker.join().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_component_reentry_keeps_replacement_map() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    import(&fixture, 1);
    let a = loaded(&fixture.server.map_manager, 0).await;
    let b = loaded(&fixture.server.map_manager, 1).await;
    let frame = place(&fixture, 0);
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    let armed = Arc::new(AtomicBool::new(false));
    let mut item = stack(0);
    item.set_data_component(ReplaceOnClone {
        action: CloneAction::Replace,
        after_clear: false,
        player: Arc::downgrade(&fixture.player.player),
        data: CustomDataImpl {
            data: NbtCompound::new(),
        },
        target: Arc::downgrade(&frame),
        armed: armed.clone(),
    });
    concrete.set_item(item, false);
    armed.store(true, Ordering::SeqCst);
    bounded_attack(&fixture, &frame);
    assert!(!armed.load(Ordering::SeqCst));
    assert_eq!(
        concrete
            .get_item()
            .get_data_component::<pumpkin_data::data_component_impl::MapIdImpl>()
            .unwrap()
            .id,
        1
    );
    assert!(!frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    markers(&a, true);
    markers(&b, false);
    persisted(&fixture, true).await;
    fixture.finish().await;
}

fn item_nbt(item: &pumpkin_data::item_stack::ItemStack) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    let mut encoded = NbtCompound::new();
    item.write_item_stack(&mut encoded);
    nbt.put_compound("Item", encoded);
    nbt.put_byte("Facing", 2);
    nbt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_same_map_nbt_metadata_change_preserves_live_marker() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    frame.read_custom_nbt(&item_nbt(&stack(0)));
    markers(&map, false);
    let mut renamed = stack(0);
    renamed.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("changed"),
    });
    frame.read_custom_nbt(&item_nbt(&renamed));
    assert_eq!(
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .get_item()
            .item,
        &Item::FILLED_MAP
    );
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    fixture.tick(11);
    markers(&map, false);
    persisted(&fixture, false).await;
    attack(&fixture.player.player, &fixture.server, frame.as_ref());
    markers(&map, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_cold_metadata_edits_preserve_live_marker() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let frame = place(&fixture, 0);
    let (entered, release) = fixture.server.map_manager.pause_next_load_for_test();
    let mut renamed = stack(0);
    renamed.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("changed"),
    });
    let nbt = item_nbt(&renamed);
    frame.read_custom_nbt(&nbt);
    assert!(fixture.server.map_manager.get_map(0).is_none());
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(fixture.server.map_manager.maps.is_empty());
    frame.read_custom_nbt(&nbt);
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, fixture.server.map_manager.drain())
        .await
        .unwrap();
    markers(&loaded(&fixture.server.map_manager, 0).await, false);
    persisted(&fixture, false).await;
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_drop_policy_observes_component_callbacks() {
    for (action, initial_drops, drops, removed) in [
        (CloneAction::Creative, true, 0, true),
        (CloneAction::DisableDrops, true, 0, false),
        (CloneAction::EnableDrops, false, 1, true),
    ] {
        let fixture = Fixture::new();
        import(&fixture, 0);
        let map = loaded(&fixture.server.map_manager, 0).await;
        let frame = place(&fixture, 0);
        fixture.world.level_info.rcu(|info| {
            let mut info = (**info).clone();
            info.game_rules.entity_drops = initial_drops;
            info
        });
        let armed = Arc::new(AtomicBool::new(false));
        let mut item = stack(0);
        item.set_data_component(ReplaceOnClone {
            action,
            after_clear: true,
            player: Arc::downgrade(&fixture.player.player),
            data: CustomDataImpl {
                data: NbtCompound::new(),
            },
            target: Arc::downgrade(&frame),
            armed: armed.clone(),
        });
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .set_item(item, false);
        armed.store(true, Ordering::SeqCst);
        bounded_attack(&fixture, &frame);
        assert!(
            !armed.load(Ordering::SeqCst),
            "callback must run after content clear"
        );
        assert_eq!(
            fixture.drops(&Item::FILLED_MAP),
            drops,
            "drop policy read before component callback"
        );
        assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
        assert!(!frame.get_entity().is_removed());
        markers(&map, removed);
        fixture.finish().await;
    }
}
