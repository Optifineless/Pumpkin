#![expect(
    clippy::unwrap_used,
    reason = "Real server regressions require valid fixtures and bounded completion"
)]

use super::{hanging_test_support::Fixture, item_frame::ItemFrameEntity};
use crate::{
    entity::EntityBase,
    plugin::{BoxFuture, EventHandler, EventPriority, player::PlayerInteractEntityEvent},
    server::Server,
    world::map::{MapData, MapManager},
};
use pumpkin_data::{
    Block, BlockDirection, data_component_impl::MapIdImpl, item::Item, item_stack::ItemStack,
    map_decoration::MapDecorationType,
};
use pumpkin_nbt::{Nbt, compound::NbtCompound, deserializer::NbtReadHelperJava, tag::NbtTag};
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    java::server::play::{SAttack, SInteract, SPlayerAction},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::{
    GameMode,
    math::{position::BlockPos, vector3::Vector3},
    text::TextComponent,
};
use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SUPPORT: BlockPos = BlockPos::new(8, 65, 10);
const DEADLINE: Duration = Duration::from_secs(30);
const IMPORTED: &[u8] = include_bytes!("../../../tests/fixtures/maps/frame-detach.dat");

fn import(fixture: &Fixture, id: i32) {
    let folder = fixture
        .server
        .basic_config
        .get_world_path()
        .join("data/minecraft/maps");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(format!("{id}.dat")), IMPORTED).unwrap();
}

fn stack(id: i32) -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::FILLED_MAP);
    stack.set_data_component(MapIdImpl { id });
    stack
}

fn place(fixture: &Fixture, id: i32) -> Arc<dyn EntityBase> {
    fixture.set_block(SUPPORT, &Block::STONE);
    let frame = fixture.place(&Item::ITEM_FRAME, SUPPORT, BlockDirection::North, None);
    // The imported record's entity ID is from a previous process, not this frame.
    assert_ne!(frame.get_entity().entity_id, 73);
    let mut nbt = NbtCompound::new();
    let mut item = NbtCompound::new();
    stack(id).write_item_stack(&mut item);
    nbt.put_compound("Item", item);
    nbt.put_byte("Facing", 2);
    frame.read_custom_nbt(&nbt);
    assert_eq!(frame.get_entity().block_pos.load(), BlockPos::new(8, 65, 9));
    frame
}

async fn loaded(manager: &MapManager, id: i32) -> Arc<Mutex<MapData>> {
    tokio::time::timeout(DEADLINE, manager.load_map(id))
        .await
        .unwrap()
        .unwrap()
}

fn dig(fixture: &Fixture) {
    fixture.player.client().handle_player_action(
        &fixture.player.player,
        &SPlayerAction {
            status: 2.into(),
            position: SUPPORT,
            face: 1,
            sequence: 1.into(),
        },
        &fixture.server,
    );
    assert!(
        fixture.world.get_block_state(&SUPPORT).is_air(),
        "real dig must remove support"
    );
}

fn attack(fixture: &Fixture, frame: &dyn EntityBase) {
    fixture
        .player
        .player
        .last_attacked_ticks
        .store(100, Ordering::Relaxed);
    let packet = SAttack {
        entity_id: frame.get_entity().entity_id.into(),
    };
    let mut bytes = Vec::new();
    packet
        .write_packet_data(&mut bytes, &pumpkin_data::packet::CURRENT_MC_VERSION)
        .unwrap();
    fixture
        .player
        .client()
        .handle_play_packet(
            &fixture.player.player,
            &fixture.server,
            &RawPacket {
                id: SAttack::to_id(pumpkin_data::packet::CURRENT_MC_VERSION),
                payload: bytes.into(),
            },
        )
        .unwrap();
    assert!(!fixture.player.client().is_closed());
}

fn markers(map: &Arc<Mutex<MapData>>, removed: bool) {
    let map = map.lock().unwrap();
    assert_eq!(&map.colors[..3], &[231, 17, 0], "unrelated pixels changed");
    assert_eq!(map.dimension.minecraft_name, "minecraft:overworld");
    assert_eq!((map.center_x, map.center_z, map.scale), (0, 0, 0));
    assert_eq!(
        map.frames.len(),
        if removed { 1 } else { 2 },
        "old saved FRAME record persists"
    );
    assert_eq!(map.banners.len(), 1);
    assert_eq!(
        map.decorations.len(),
        if removed { 2 } else { 3 },
        "old FRAME icon persists"
    );
    let banner = &map.decorations[0];
    assert_eq!(banner.icon_type, MapDecorationType::BANNER_RED.id as i32);
    assert_eq!(
        banner.display_name,
        Some(TextComponent::text("Home").bold())
    );
    let other = map.frames.last().unwrap().extract_compound().unwrap();
    assert_eq!(other.get_int("entity_id"), Some(74));
    assert_eq!(other.get_string("frame_extra"), Some("retained"));
}

fn unknown_data(fixture: &Fixture) {
    let path = fixture
        .server
        .basic_config
        .get_world_path()
        .join("data/minecraft/maps/0.dat");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    let root = Nbt::read(&mut NbtReadHelperJava::new(&mut std::io::Cursor::new(
        bytes,
    )))
    .unwrap()
    .root_tag;
    assert_eq!(root.get_string("root_extra"), Some("retained"));
    assert_eq!(
        root.get_compound("data").unwrap().get_string("map_extra"),
        Some("retained")
    );
}

async fn persisted(fixture: &Fixture, removed: bool) {
    tokio::time::timeout(DEADLINE, fixture.server.save_maps())
        .await
        .unwrap()
        .unwrap();
    let manager = MapManager::load(&fixture.server.basic_config.get_world_path()).unwrap();
    let reloaded = loaded(&manager, 0).await;
    assert_eq!(
        reloaded.lock().unwrap().frames.len(),
        if removed { 1 } else { 2 },
        "saved frame record survives real save/reload"
    );
    markers(&reloaded, removed);
    unknown_data(fixture);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_support_loss_unlinks_saved_frame_marker() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    markers(&map, false);
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed(), "supported control");
    map.lock().unwrap().dirty = false;
    dig(&fixture);
    fixture.tick(101);
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 1);
    assert!(
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .get_item()
            .is_empty()
    );
    persisted(&fixture, true).await;
    markers(&map, true);
    assert!(
        map.lock().unwrap().dirty,
        "marker cleanup must dirty the map"
    );
    fixture.finish().await;
}

struct CancelAttack(AtomicBool);
impl EventHandler<PlayerInteractEntityEvent> for CancelAttack {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerInteractEntityEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.0.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_creative_and_no_drop_marker_cleanup() {
    for environmental in [false, true] {
        let fixture = Fixture::new();
        import(&fixture, 0);
        let map = loaded(&fixture.server.map_manager, 0).await;
        let frame = place(&fixture, 0);
        let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
        let cancel = Arc::new(CancelAttack(AtomicBool::new(true)));
        fixture
            .server
            .plugin_manager
            .register::<PlayerInteractEntityEvent, _>(cancel.clone(), EventPriority::Normal, true);
        fixture.player.client().handle_interact(
            &fixture.player.player,
            &SInteract {
                entity_id: frame.get_entity().entity_id.into(),
                r#type: 1.into(),
                target_position: None,
                hand: None,
                sneaking: false,
            },
            &fixture.server,
        );
        assert_eq!(
            concrete.get_item().item,
            &Item::FILLED_MAP,
            "cancelled damage retains content"
        );
        markers(&map, false);
        cancel.0.store(false, Ordering::Relaxed);
        concrete.set_fixed(true);
        attack(&fixture, frame.as_ref());
        dig(&fixture);
        fixture.tick(101);
        assert!(!frame.get_entity().is_removed(), "Fixed control");
        assert_eq!(concrete.get_item().item, &Item::FILLED_MAP);
        markers(&map, false);
        concrete.set_fixed(false);
        if environmental {
            fixture.world.level_info.rcu(|info| {
                let mut info = (**info).clone();
                info.game_rules.entity_drops = false;
                info
            });
            fixture.tick(101);
        } else {
            assert!(fixture.player.player.set_gamemode(GameMode::Creative));
            attack(&fixture, frame.as_ref());
            assert!(
                !frame.get_entity().is_removed(),
                "first Creative hit pops content"
            );
            assert!(concrete.get_item().is_empty());
            attack(&fixture, frame.as_ref());
        }
        assert!(frame.get_entity().is_removed());
        assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
        assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
        markers(&map, true);
        persisted(&fixture, true).await;
        fixture.finish().await;
    }
    // Supplied dropItem deliberately leaves a player-caused no-drop content pop alone.
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    fixture.world.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.entity_drops = false;
        info
    });
    attack(&fixture, frame.as_ref());
    assert!(
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .get_item()
            .is_empty()
    );
    assert!(!frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    markers(&map, false);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_cold_map_and_replacement_keep_cleanup_obligation() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    import(&fixture, 1);
    let frame = place(&fixture, 0);
    assert!(
        fixture.server.map_manager.maps.is_empty(),
        "real stored map starts cold"
    );
    dig(&fixture);
    fixture.tick(101);
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 1);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 1);
    // A different frame at the same attachment displays B while A's load may still be queued.
    let replacement = place(&fixture, 1);
    assert!(!replacement.get_entity().is_removed());
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    tokio::time::timeout(DEADLINE, fixture.server.shutdown())
        .await
        .unwrap();
    let manager = MapManager::load(&fixture.server.basic_config.get_world_path()).unwrap();
    markers(&loaded(&manager, 0).await, true);
    markers(&loaded(&manager, 1).await, false);
    unknown_data(&fixture);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_same_attachment_replacement_rejects_stale_cleanup() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let frame = place(&fixture, 0);
    let (entered, release) = fixture.server.map_manager.pause_next_load_for_test();
    attack(&fixture, frame.as_ref());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(fixture.server.map_manager.maps.is_empty());
    attack(&fixture, frame.as_ref());
    assert!(frame.get_entity().is_removed());
    let replacement = place(&fixture, 0);
    let old = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    // Reentry through a retained old instance cannot withdraw the new owner.
    old.set_item(stack(0), false);
    assert!(old.get_item().is_empty());
    release.send(()).unwrap();
    tokio::time::timeout(DEADLINE, fixture.server.map_manager.drain())
        .await
        .unwrap();
    let map = loaded(&fixture.server.map_manager, 0).await;
    markers(&map, false);
    assert_eq!(
        replacement
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .get_item()
            .item,
        &Item::FILLED_MAP
    );
    persisted(&fixture, false).await;
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_live_nbt_and_kill_retain_original_attachment() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    import(&fixture, 1);
    let a = loaded(&fixture.server.map_manager, 0).await;
    let b = loaded(&fixture.server.map_manager, 1).await;
    let frame = place(&fixture, 0);
    let mut nbt = NbtCompound::new();
    frame.write_nbt(&mut nbt);
    let mut item = NbtCompound::new();
    stack(1).write_item_stack(&mut item);
    nbt.put_compound("Item", item);
    nbt.put_list(
        "Pos",
        vec![
            NbtTag::Double(10.5),
            NbtTag::Double(65.5),
            NbtTag::Double(9.5),
        ],
    );
    frame.read_nbt_non_mut(&nbt);
    assert_eq!(
        frame.get_entity().block_pos.load(),
        BlockPos::new(10, 65, 9)
    );
    markers(&a, true);
    markers(&b, false);
    // An absent Item in the real load chain clears the old display.
    nbt.child_tags.remove("Item");
    frame.read_nbt_non_mut(&nbt);
    assert!(
        frame
            .cast_any()
            .downcast_ref::<ItemFrameEntity>()
            .unwrap()
            .get_item()
            .is_empty()
    );
    // Restore the original attachment, then /kill a Fixed map frame without drops.
    frame.get_entity().set_pos(Vector3::new(8.5, 65.5, 9.5));
    frame
        .cast_any()
        .downcast_ref::<ItemFrameEntity>()
        .unwrap()
        .set_item(stack(1), false);
    frame
        .cast_any()
        .downcast_ref::<ItemFrameEntity>()
        .unwrap()
        .set_fixed(true);
    fixture
        .server
        .worlds
        .store(Arc::new(vec![fixture.world.clone()]));
    let source = crate::command::CommandSender::Console.into_source(&fixture.server);
    assert_eq!(
        fixture
            .server
            .command_dispatcher
            .load()
            .execute_input(&format!("kill {}", frame.get_entity().entity_uuid), &source)
            .unwrap(),
        1
    );
    assert!(frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    markers(&b, true);
    persisted(&fixture, true).await;
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_supported_fixed_and_cancelled_controls() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed());
    let cancel = Arc::new(CancelAttack(AtomicBool::new(true)));
    fixture
        .server
        .plugin_manager
        .register::<PlayerInteractEntityEvent, _>(cancel, EventPriority::Normal, true);
    fixture.player.client().handle_interact(
        &fixture.player.player,
        &SInteract {
            entity_id: frame.get_entity().entity_id.into(),
            r#type: 1.into(),
            target_position: None,
            hand: None,
            sneaking: false,
        },
        &fixture.server,
    );
    let concrete = frame.cast_any().downcast_ref::<ItemFrameEntity>().unwrap();
    assert_eq!(concrete.get_item().item, &Item::FILLED_MAP);
    concrete.set_fixed(true);
    dig(&fixture);
    fixture.tick(101);
    assert!(!frame.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::ITEM_FRAME), 0);
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 0);
    markers(&map, false);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g04_unrelated_runtime_decoration_survives_cleanup() {
    let fixture = Fixture::new();
    import(&fixture, 0);
    let map = loaded(&fixture.server.map_manager, 0).await;
    let frame = place(&fixture, 0);
    map.lock()
        .unwrap()
        .decorations
        .push(crate::world::map::MapDecoration {
            icon_type: MapDecorationType::RED_MARKER.id as i32,
            x: 45,
            z: 46,
            direction: 3,
            display_name: Some(TextComponent::text("plugin marker")),
        });
    attack(&fixture, frame.as_ref());
    assert_eq!(fixture.drops(&Item::FILLED_MAP), 1);
    let retained = {
        let map = map.lock().unwrap();
        assert_eq!(map.frames.len(), 1);
        assert_eq!(map.decorations.len(), 3);
        map.decorations.last().unwrap().clone()
    };
    assert_eq!(
        retained.display_name,
        Some(TextComponent::text("plugin marker"))
    );
    assert_eq!((retained.x, retained.z, retained.direction), (45, 46, 3));
    fixture.finish().await;
}

#[path = "frame_map_lifecycle_tests.rs"]
mod lifecycle;
