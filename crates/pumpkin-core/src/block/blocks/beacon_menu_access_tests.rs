use super::*;
use crate::block::entities::{BlockEntity, block_entity_from_nbt};
use pumpkin_data::{
    data_component_impl::{CustomDataImpl, CustomNameImpl, LockImpl},
    screen::WindowType,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::ser::{NetworkReadExt, NetworkReadSliceExt};
use pumpkin_util::{GameMode, text::TextComponent};
use std::time::Duration;

fn styled_name(text: &str, color: &str) -> NbtTag {
    let mut name = NbtCompound::new();
    name.put_string("text", text.to_owned());
    name.put_string("color", color.to_owned());
    name.put_bool("bold", true);
    let mut child = NbtCompound::new();
    child.put_string("text", "!".to_owned());
    child.put_bool("italic", true);
    name.put_list("extra", vec![NbtTag::Compound(child)]);
    NbtTag::Compound(name)
}

fn named_key(name: &NbtTag) -> ItemStack {
    let mut key = ItemStack::new(1, &Item::PAPER);
    key.set_data_component(CustomNameImpl::read_data(name).unwrap());
    key
}

fn canonical_beacon(title: &NbtTag, key: &NbtTag) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put("CustomName", title.clone());
    let mut components = NbtCompound::new();
    components.put("minecraft:custom_name", key.clone());
    let mut predicate = NbtCompound::new();
    predicate.put_compound("components", components);
    nbt.put_compound("lock", predicate);
    nbt
}

fn custom_data_beacon(title: &NbtTag, key: &NbtTag) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put("CustomName", title.clone());
    let mut predicates = NbtCompound::new();
    predicates.put("minecraft:custom_data", key.clone());
    let mut predicate = NbtCompound::new();
    predicate.put_compound("predicates", predicates);
    nbt.put_compound("lock", predicate);
    nbt
}

fn custom_data_key(beacon_key: NbtTag) -> ItemStack {
    let mut data = NbtCompound::new();
    data.put("beacon_key", beacon_key);
    data.put_string("extra", "allowed subset data".to_owned());
    let mut nested_entry = NbtCompound::new();
    nested_entry.put_string("id", "matching entry".to_owned());
    nested_entry.put_bool("extra", true);
    let mut other_entry = NbtCompound::new();
    other_entry.put_string("id", "unrelated entry".to_owned());
    let mut nested = NbtCompound::new();
    nested.put_list(
        "entries",
        vec![
            NbtTag::Compound(other_entry),
            NbtTag::Compound(nested_entry),
        ],
    );
    nested.put_string("extra", "nested subset data".to_owned());
    data.put_compound("nested", nested);
    let mut key = ItemStack::new(1, &Item::PAPER);
    key.set_data_component(CustomDataImpl::new(data));
    key
}

fn expected_custom_data() -> NbtCompound {
    let mut list_entry = NbtCompound::new();
    list_entry.put_string("id", "matching entry".to_owned());
    let mut nested = NbtCompound::new();
    nested.put_list("entries", vec![NbtTag::Compound(list_entry)]);
    let mut data = NbtCompound::new();
    data.put_int("beacon_key", 1);
    data.put_compound("nested", nested);
    data
}

fn assert_custom_data_beacon_denied(
    fixture: &mut Fixture,
    title: &TextComponent,
    main_hand: ItemStack,
    offhand: ItemStack,
    control: &str,
) {
    fixture.player.player.inventory.set_stack(0, main_hand);
    fixture.player.player.inventory.set_stack(
        pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
        offhand,
    );
    assert_denied(fixture, title, control);
}

fn open_custom_data_chest(fixture: &mut Fixture, expected_data: &NbtTag) {
    let chest_pos = pumpkin_util::math::position::BlockPos::new(1, 64, 0);
    fixture.world.set_block_state(
        &chest_pos,
        Block::CHEST.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE
            | pumpkin_world::world::BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    let mut chest_nbt = custom_data_beacon(&NbtTag::String("unused".into()), expected_data);
    chest_nbt.put_string(
        "id",
        crate::block::entities::chest::ChestBlockEntity::ID.to_owned(),
    );
    chest_nbt.put_int("x", chest_pos.0.x);
    chest_nbt.put_int("y", chest_pos.0.y);
    chest_nbt.put_int("z", chest_pos.0.z);
    let chest =
        block_entity_from_nbt(&chest_nbt).expect("load a chest through the production factory");
    fixture.world.add_block_entity(chest);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, custom_data_key(NbtTag::Int(1)));
    fixture.use_block(&fixture.player, chest_pos);
    let player_menu: SharedScreenHandler = fixture.player.player.player_screen_handler.clone();
    assert!(
        !Arc::ptr_eq(&current_screen(&fixture.player.player), &player_menu),
        "matching custom_data must open a chest through its real use path"
    );
    assert!(has_packet(
        &fixture.player.take_packets(),
        pumpkin_data::packet::clientbound::play::OPEN_SCREEN.0
    ));
    fixture.player.player.close_handled_screen();
}

fn load_beacon(fixture: &Fixture, mut nbt: NbtCompound) -> Arc<dyn BlockEntity> {
    nbt.put_string("id", BeaconBlockEntity::ID.to_owned());
    nbt.put_int("x", fixture.pos.0.x);
    nbt.put_int("y", fixture.pos.0.y);
    nbt.put_int("z", fixture.pos.0.z);
    let entity = block_entity_from_nbt(&nbt).expect("load a beacon through the production factory");
    fixture.world.add_block_entity(entity.clone());
    entity
}

fn save_reload(fixture: &Fixture, entity: &dyn BlockEntity) -> Arc<dyn BlockEntity> {
    let mut saved = NbtCompound::new();
    entity.write_internal(&mut saved);
    load_beacon(fixture, saved)
}

fn has_packet(packets: &[bytes::Bytes], packet_id: i32) -> bool {
    packets.iter().any(|packet| {
        let mut reader = packet.as_ref();
        reader.get_var_int().unwrap().0 == packet_id
    })
}

fn assert_open_title(fixture: &mut Fixture, title: &TextComponent) {
    assert_beacon_menu(&fixture.player.player);
    let packets = fixture.player.take_packets();
    let opened = packets
        .iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            (reader.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::OPEN_SCREEN.0)
                .then_some(reader)
        })
        .collect::<Vec<_>>();
    assert_eq!(opened.len(), 1, "one real OpenScreen must be published");
    let mut reader = opened[0];
    assert_eq!(
        reader.get_var_int().unwrap().0,
        i32::from(menu_sync_id(&fixture.player.player))
    );
    assert_eq!(reader.get_var_int().unwrap().0, WindowType::Beacon as i32);
    assert_eq!(
        reader
            .get_nbt(&pumpkin_data::packet::CURRENT_MC_VERSION)
            .unwrap(),
        Some(title.to_nbt_tag_for_version(&pumpkin_data::packet::CURRENT_MC_VERSION)),
        "the real OpenScreen packet must retain the beacon title component",
    );
    assert!(reader.is_empty());
    assert!(
        !has_packet(
            &packets,
            pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0
        ),
        "an admitted beacon must not send the locked notification"
    );
    fixture.player.player.close_handled_screen();
    fixture.player.take_packets();
}

fn assert_denied(fixture: &mut Fixture, title: &TextComponent, control: &str) {
    fixture.player.take_packets();
    fixture.use_beacon();
    let player_menu: SharedScreenHandler = fixture.player.player.player_screen_handler.clone();
    assert!(
        Arc::ptr_eq(&current_screen(&fixture.player.player), &player_menu),
        "{control}: a locked beacon must not install a menu"
    );
    let notification = TextComponent::translate("container.isLocked", [title.clone()]);
    let packets = fixture.player.take_packets();
    assert!(
        !has_packet(
            &packets,
            pumpkin_data::packet::clientbound::play::OPEN_SCREEN.0
        ),
        "{control}: denied access must not publish OpenScreen"
    );
    let messages = packets
        .iter()
        .filter_map(|packet| {
            let mut reader = packet.as_ref();
            if reader.get_var_int().unwrap().0
                != pumpkin_data::packet::clientbound::play::SYSTEM_CHAT.0
            {
                return None;
            }
            let message = reader
                .get_nbt(&pumpkin_data::packet::CURRENT_MC_VERSION)
                .unwrap();
            let overlay = reader.get_bool().unwrap();
            assert!(reader.is_empty());
            Some((message, overlay))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![(
            Some(notification.to_nbt_tag_for_version(&pumpkin_data::packet::CURRENT_MC_VERSION)),
            true
        )],
        "{control}: denied access must send the existing locked notification with the display name",
    );
    assert!(
        has_packet(&packets, pumpkin_data::packet::clientbound::play::SOUND.0),
        "{control}: denied access must play the locked sound"
    );
}

fn assert_legacy_reload(fixture: &mut Fixture) {
    let mut old_save = NbtCompound::new();
    old_save.put_string("Lock", "old fork key".to_owned());
    old_save.put_string("CustomName", "Legacy beacon".to_owned());
    let entity = load_beacon(fixture, old_save);
    let _reloaded = save_reload(fixture, entity.as_ref());
    fixture.player.player.gamemode.store(GameMode::Survival);
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::EMPTY.clone());
    let title = TextComponent::text("Legacy beacon");
    assert_denied(
        fixture,
        &title,
        "uppercase Lock survives canonical save and reload",
    );
    fixture
        .player
        .player
        .inventory
        .set_stack(0, named_key(&NbtTag::String("old fork key".into())));
    fixture.use_beacon();
    assert_open_title(fixture, &title);
}

fn assert_lock_components_apply_and_clear(fixture: &mut Fixture) {
    let title_tag = styled_name("Placed beacon", "gold");
    let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
    let key_tag = styled_name("Placed key", "red");
    let nbt = canonical_beacon(&title_tag, &key_tag);
    let expected_lock = LockImpl::read_data(nbt.get("lock").unwrap()).unwrap();
    let entity = load_beacon(fixture, nbt);
    let mut stack = ItemStack::new(1, &Item::BEACON);
    entity.collect_item_components(&mut stack);
    assert_eq!(stack.get_data_component::<LockImpl>(), Some(&expected_lock));

    let placed = load_beacon(fixture, NbtCompound::new());
    placed.apply_item_components(&stack);
    let placed = save_reload(fixture, placed.as_ref());
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::EMPTY.clone());
    assert_denied(
        fixture,
        &title,
        "placed lock component remains enforced after reload",
    );
    fixture
        .player
        .player
        .inventory
        .set_stack(0, named_key(&key_tag));
    fixture.use_beacon();
    assert_open_title(fixture, &title);

    placed.apply_item_components(&ItemStack::new(1, &Item::BEACON));
    let mut cleared = ItemStack::new(1, &Item::BEACON);
    placed.collect_item_components(&mut cleared);
    assert!(cleared.get_data_component::<LockImpl>().is_none());
    assert!(cleared.get_data_component::<CustomNameImpl>().is_none());
    fixture
        .player
        .player
        .inventory
        .set_stack(0, ItemStack::EMPTY.clone());
    fixture.use_beacon();
    assert_open_title(fixture, &TextComponent::translate("container.beacon", []));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_lock_and_named_title() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Vault beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let key_tag = styled_name("Vault key", "red");
        let entity = load_beacon(&fixture, canonical_beacon(&title_tag, &key_tag));
        let entity = save_reload(&fixture, entity.as_ref());
        for (control, main, offhand) in [
            (
                "empty key",
                ItemStack::EMPTY.clone(),
                ItemStack::EMPTY.clone(),
            ),
            (
                "same text with wrong style",
                named_key(&styled_name("Vault key", "blue")),
                ItemStack::EMPTY.clone(),
            ),
            (
                "matching key only in offhand",
                named_key(&NbtTag::String("wrong key".into())),
                named_key(&key_tag),
            ),
        ] {
            fixture.player.player.inventory.set_stack(0, main);
            fixture.player.player.inventory.set_stack(
                pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
                offhand,
            );
            assert_denied(&mut fixture, &title, control);
        }

        let mut saved = NbtCompound::new();
        entity.write_internal(&mut saved);
        assert_eq!(saved.get("CustomName"), Some(&title_tag));
        assert!(saved.get_compound("lock").is_some());

        fixture
            .player
            .player
            .inventory
            .set_stack(0, named_key(&key_tag));
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        fixture
            .player
            .player
            .inventory
            .set_stack(0, ItemStack::EMPTY.clone());
        fixture.player.player.gamemode.store(GameMode::Spectator);
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        assert_legacy_reload(&mut fixture);
        assert_lock_components_apply_and_clear(&mut fixture);
        fixture.finish().await;
    })
    .await
    .expect("bounded beacon access regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_named_title_and_component_controls() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let component_list = vec![
            styled_name("First", "green"),
            NbtTag::String(" second".into()),
        ];
        for tag in [
            styled_name("Named beacon", "aqua"),
            NbtTag::List(component_list),
            NbtTag::String("old fork name".into()),
        ] {
            let title = CustomNameImpl::read_data(&tag).unwrap().name;
            let mut nbt = NbtCompound::new();
            nbt.put("CustomName", tag);
            let entity = load_beacon(&fixture, nbt);
            let reloaded = save_reload(&fixture, entity.as_ref());
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);

            let mut stack = ItemStack::new(1, &Item::BEACON);
            reloaded.collect_item_components(&mut stack);
            let expected = CustomNameImpl {
                name: title.clone(),
            };
            assert_eq!(
                stack.get_data_component::<CustomNameImpl>(),
                Some(&expected)
            );
            let placed = load_beacon(&fixture, NbtCompound::new());
            placed.apply_item_components(&stack);
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);
        }
        fixture.finish().await;
    })
    .await
    .expect("bounded beacon access regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_unlocked_default_title_control() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let entity = load_beacon(&fixture, NbtCompound::new());
        save_reload(&fixture, entity.as_ref());
        fixture.use_beacon();
        assert_open_title(
            &mut fixture,
            &TextComponent::translate("container.beacon", []),
        );
        fixture.finish().await;
    })
    .await
    .expect("bounded beacon access regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_beacon_native_lock_updates_after_reload() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Reloaded beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        for canonical_only in [true, false] {
            let original_key = if canonical_only {
                styled_name("Original key", "red")
            } else {
                NbtTag::String("Original key".into())
            };
            let nbt = if canonical_only {
                canonical_beacon(&title_tag, &original_key)
            } else {
                let mut nbt = NbtCompound::new();
                nbt.put("CustomName", title_tag.clone());
                nbt.put_string("Lock", "Original key".to_owned());
                nbt
            };
            let loaded = load_beacon(&fixture, nbt);
            let reloaded = save_reload(&fixture, loaded.as_ref());
            let beacon = reloaded
                .as_any()
                .downcast_ref::<BeaconBlockEntity>()
                .unwrap();

            fixture
                .player
                .player
                .inventory
                .set_stack(0, ItemStack::EMPTY.clone());
            assert_denied(&mut fixture, &title, "unchanged reload preserves its lock");
            fixture
                .player
                .player
                .inventory
                .set_stack(0, named_key(&original_key));
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);

            *beacon.lock_key.lock().unwrap() = Some("Replacement key".to_owned());
            assert_denied(
                &mut fixture,
                &title,
                "native replacement invalidates the original key",
            );
            fixture
                .player
                .player
                .inventory
                .set_stack(0, named_key(&NbtTag::String("Replacement key".into())));
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);

            *beacon.lock_key.lock().unwrap() = None;
            fixture
                .player
                .player
                .inventory
                .set_stack(0, ItemStack::EMPTY.clone());
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);
            let unlocked = save_reload(&fixture, reloaded.as_ref());
            let mut saved = NbtCompound::new();
            unlocked.write_internal(&mut saved);
            assert!(saved.get("lock").is_none());
            assert!(saved.get("Lock").is_none());
            fixture.use_beacon();
            assert_open_title(&mut fixture, &title);
        }
        fixture.finish().await;
    })
    .await
    .expect("bounded beacon native lock update regression timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g03_custom_data_locks_open_beacons_and_chests() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = Fixture::new(0);
        let title_tag = styled_name("Custom data beacon", "gold");
        let title = CustomNameImpl::read_data(&title_tag).unwrap().name;
        let expected_data = expected_custom_data();
        let beacon = load_beacon(
            &fixture,
            custom_data_beacon(&title_tag, &NbtTag::Compound(expected_data.clone())),
        );
        let beacon = save_reload(&fixture, beacon.as_ref());
        let mut saved = NbtCompound::new();
        beacon.write_internal(&mut saved);
        assert_eq!(
            saved
                .get_compound("lock")
                .unwrap()
                .get_compound("predicates")
                .unwrap()
                .get_compound("minecraft:custom_data"),
            Some(&expected_data)
        );

        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Int(2)),
            ItemStack::EMPTY.clone(),
            "wrong custom_data key",
        );
        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Byte(1)),
            ItemStack::EMPTY.clone(),
            "same number with a different NBT tag type",
        );
        assert_custom_data_beacon_denied(
            &mut fixture,
            &title,
            custom_data_key(NbtTag::Int(2)),
            custom_data_key(NbtTag::Int(1)),
            "matching custom_data key only in offhand",
        );

        fixture
            .player
            .player
            .inventory
            .set_stack(0, custom_data_key(NbtTag::Int(1)));
        fixture.player.player.inventory.set_stack(
            pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
            ItemStack::EMPTY.clone(),
        );
        fixture.use_beacon();
        assert_open_title(&mut fixture, &title);

        open_custom_data_chest(&mut fixture, &NbtTag::Compound(expected_data));
        fixture.finish().await;
    })
    .await
    .expect("bounded custom_data container lock regression timed out");
}

#[path = "beacon_custom_data_access_tests.rs"]
mod custom_data_access_tests;
