use super::support::{Fixture, Item, ItemStack, NbtCompound, TestPlayer, equipment};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, DataComponentImpl, EquipmentSlot, ItemNameImpl,
        Rarity, RarityImpl, TooltipDisplayImpl,
    },
    dye_color::DyeColor,
    packet::CURRENT_MC_VERSION,
};
use pumpkin_inventory::Inventory;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{
    ServerPacket, VarInt, java::server::play::SSetCreativeSlot, ser::NetworkWriteExt,
};

fn creative(components: &[(u8, &[u8])]) -> Vec<u8> {
    // Slot 36; one white_banner (1417); bounded, length-prefixed component patch.
    let mut bytes = vec![0, 36, 1, 0x89, 0x0b, components.len() as u8, 0];
    for (id, payload) in components {
        bytes.push(*id);
        bytes.write_var_int(&VarInt(payload.len() as i32)).unwrap();
        bytes.extend_from_slice(payload);
    }
    bytes
}

fn direct_payload() -> Vec<u8> {
    // BannerPattern.DIRECT_STREAM_CODEC: Identifier then UTF-8 translation key, then dye.
    let mut bytes = vec![1, 0, 23];
    bytes.extend_from_slice(b"minecraft:custom/banner");
    bytes.push(14);
    bytes.extend_from_slice(b"example.banner");
    bytes.push(9);
    bytes
}

pub(super) fn direct_stack() -> ItemStack {
    let mut stack = ItemStack::new(1, &Item::WHITE_BANNER);
    stack.patch.push((
        DataComponent::BannerPatterns,
        Some(
            BannerPatternsImpl {
                layers: vec![BannerPatternLayer {
                    pattern: "minecraft:custom/banner".into(),
                    translation_key: Some("example.banner".into()),
                    color: DyeColor::Cyan,
                }],
            }
            .to_dyn(),
        ),
    ));
    stack
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_holder_wire_and_persistent_forms_are_native() {
    Fixture::new()
        .verify(|fixture| {
            fixture
                .client
                .player
                .set_gamemode(pumpkin_util::GameMode::Creative);
            let mut observer = TestPlayer::new(&fixture.world);
            fixture.world.players.store(std::sync::Arc::new(vec![
                fixture.client.player.clone(),
                observer.player.clone(),
            ]));
            let tracked = fixture
                .world
                .entity_tracker
                .get_tracked_entity(fixture.client.player.entity_id())
                .unwrap();
            tracked.seen_by.insert(observer.player.gameprofile.id);
            observer.take_packets();
            let direct = direct_payload();
            for (pattern, is_direct) in [(&[1, 2, 15][..], false), (direct.as_slice(), true)] {
                let bytes = creative(&[(74, pattern), (18, &[0, 2, 74, 9]), (12, &[1])]);
                let mut raw = bytes.as_slice();
                let packet = SSetCreativeSlot::read_bounded(&mut raw, &CURRENT_MC_VERSION);
                assert!(
                    packet.is_ok(),
                    "independent vanilla reference/direct Creative fixture must decode: {:?}",
                    packet.as_ref().err()
                );
                let packet = packet.unwrap();
                assert!(
                    raw.is_empty(),
                    "bounded Creative packet consumed every component byte"
                );
                fixture
                    .client
                    .client()
                    .handle_set_creative_slot(&fixture.client.player, packet)
                    .unwrap();
                fixture.client.player.tick(&fixture.server);
                let admitted = fixture.client.player.inventory.get_stack(0);
                assert_admitted(&admitted, is_direct);
                let published = equipment(&mut observer, fixture.client.player.entity_id());
                assert!(published[&EquipmentSlot::MAIN_HAND.discriminant()].are_equal(&admitted));
                persistent_republication(fixture, &mut observer, &admitted, is_direct);
            }
            let native = native_direct_saved_fixture();
            fixture
                .client
                .client()
                .handle_set_creative_slot(
                    &fixture.client.player,
                    SSetCreativeSlot::new(36, native.clone().into()),
                )
                .unwrap();
            assert!(
                fixture
                    .client
                    .player
                    .inventory
                    .get_stack(0)
                    .are_equal(&native)
            );
            persistent_republication(fixture, &mut observer, &native, true);
            malformed_controls();
        })
        .await;
}

fn assert_admitted(admitted: &ItemStack, is_direct: bool) {
    let layer = &admitted
        .get_data_component::<BannerPatternsImpl>()
        .unwrap()
        .layers[0];
    if is_direct {
        assert_eq!(layer.pattern, "minecraft:custom/banner");
        assert_eq!(layer.translation_key.as_deref(), Some("example.banner"));
        assert_eq!(layer.color, DyeColor::Cyan);
    } else {
        assert_eq!(layer.pattern, "minecraft:border");
        assert_eq!(layer.translation_key, None);
        assert_eq!(layer.color, DyeColor::Black);
    }
    assert!(
        admitted
            .get_data_component::<TooltipDisplayImpl>()
            .unwrap()
            .hidden_components
            == [DataComponent::BannerPatterns, DataComponent::ItemName]
    );
    assert_eq!(
        admitted.get_data_component::<RarityImpl>().unwrap().rarity,
        Rarity::Uncommon
    );
}

fn native_direct_saved_fixture() -> ItemStack {
    let mut holder = NbtCompound::new();
    holder.put_string("asset_id", "minecraft:custom/banner".into());
    holder.put_string("translation_key", "example.banner".into());
    let mut layer = NbtCompound::new();
    layer.put_compound("pattern", holder);
    layer.put_string("color", "cyan".into());
    let mut components = NbtCompound::new();
    components.put_list("minecraft:banner_patterns", vec![NbtTag::Compound(layer)]);
    let mut saved = NbtCompound::new();
    saved.put_string("id", "minecraft:white_banner".into());
    saved.put_int("count", 1);
    saved.put_compound("components", components);
    let loaded = ItemStack::read_item_stack(&saved);
    assert!(
        loaded.is_some(),
        "valid vanilla nested direct holder must not discard its saved stack"
    );
    let loaded = loaded.unwrap();
    assert!(
        loaded.are_equal(&direct_stack()),
        "independent native direct NBT loads the complete stack"
    );
    loaded
}

fn persistent_republication(
    fixture: &Fixture,
    observer: &mut TestPlayer,
    stack: &ItemStack,
    direct: bool,
) {
    let mut nbt = NbtCompound::new();
    stack.write_item_stack(&mut nbt);
    let component = nbt
        .get_compound("components")
        .unwrap()
        .get("minecraft:banner_patterns")
        .unwrap();
    let layers = component.extract_list().unwrap();
    let holder = layers[0]
        .extract_compound()
        .unwrap()
        .get("pattern")
        .unwrap();
    if direct {
        assert!(
            holder.extract_compound().is_some(),
            "direct holder persistence must use the native nested compound"
        );
        let nested = holder.extract_compound().unwrap();
        assert_eq!(
            nested.get_string("asset_id"),
            Some("minecraft:custom/banner")
        );
        assert_eq!(nested.get_string("translation_key"), Some("example.banner"));
    } else {
        assert_eq!(holder.extract_string(), Some("minecraft:border"));
    }
    let loaded = ItemStack::read_item_stack(&nbt).unwrap();
    assert!(
        loaded.are_equal(stack),
        "valid native holder stack survives save/load"
    );
    fixture
        .client
        .player
        .inventory
        .set_stack(0, ItemStack::EMPTY.clone());
    fixture
        .client
        .client()
        .handle_set_creative_slot(
            &fixture.client.player,
            SSetCreativeSlot::new(36, loaded.clone().into()),
        )
        .unwrap();
    fixture.client.player.tick(&fixture.server);
    // Explicit tracker pairing republishes authoritative storage after reload.
    let tracked = fixture
        .world
        .entity_tracker
        .get_tracked_entity(fixture.client.player.entity_id())
        .unwrap();
    tracked.add_pairing(&observer.player);
    let published = equipment(observer, fixture.client.player.entity_id());
    assert!(published[&EquipmentSlot::MAIN_HAND.discriminant()].are_equal(&loaded));
}

fn malformed_controls() {
    let mut bad_asset = direct_payload();
    bad_asset[3] = b'M';
    let cases: Vec<Vec<u8>> = vec![
        vec![1, 0xff, 0xff, 0xff, 0xff, 0x0f, 15], // negative holder
        vec![0xff, 0xff, 0xff, 0xff, 0x0f],        // negative layer count
        vec![1, 0xff, 0x7f, 15],                   // holder beyond synchronized vanilla prefix
        vec![1, 2, 16],                            // unknown dye
        vec![1, 2, 0xff, 0xff, 0xff, 0xff, 0x0f],  // negative dye
        bad_asset,
        vec![1, 0, 0xff, 0xff, 0x7f], // huge direct string before allocation
        vec![0xff, 0xff, 0x7f],       // huge layer count
        vec![1, 0],                   // truncated direct holder
    ];
    for payload in cases {
        let bytes = creative(&[(74, &payload)]);
        assert!(
            SSetCreativeSlot::read_bounded(&mut bytes.as_slice(), &CURRENT_MC_VERSION).is_err(),
            "malformed Creative holder was accepted: {payload:?}"
        );
    }
    for payload in [
        &[0, 0xff, 0xff, 0xff, 0xff, 0x0f][..],
        &[0, 1, 0xff, 0x7f],
        &[0, 2, 74],
    ] {
        let bytes = creative(&[(18, payload), (12, &[1])]);
        assert!(
            SSetCreativeSlot::read_bounded(&mut bytes.as_slice(), &CURRENT_MC_VERSION).is_err(),
            "malformed tooltip collection"
        );
    }
    let good = creative(&[(74, &[1, 2, 15])]);
    for end in 0..good.len() {
        assert!(
            SSetCreativeSlot::read_bounded(&mut &good[..end], &CURRENT_MC_VERSION).is_err(),
            "truncated Creative payload at {end}"
        );
    }
}

pub(super) fn malformed_name_control(fixture: &Fixture) {
    fixture
        .client
        .player
        .set_gamemode(pumpkin_util::GameMode::Creative);
    let mut original_b1: &[u8] = &[0x00, 0x24, 0x01, 0x01, 0x01, 0x00, 0x09, 0x02, 0x0a, 0x00];
    let before = fixture.client.player.inventory.get_stack(0);
    let parsed = SSetCreativeSlot::read_bounded(&mut original_b1, &CURRENT_MC_VERSION);
    if let Ok(packet) = parsed {
        fixture
            .client
            .client()
            .handle_set_creative_slot(&fixture.client.player, packet)
            .unwrap();
        fixture.client.player.tick(&fixture.server);
        let normalized = fixture.client.player.inventory.get_stack(0);
        let mut saved = NbtCompound::new();
        normalized.write_item_stack(&mut saved);
        let name = saved
            .get_compound("components")
            .unwrap()
            .get_compound("minecraft:item_name")
            .unwrap();
        assert!(
            name.get_string("translate").is_some(),
            "B1 must never persist or republish invalid empty text NBT"
        );
        let reloaded = ItemStack::read_item_stack(&saved).unwrap();
        assert!(reloaded.are_equal(&normalized));
        fixture
            .client
            .client()
            .handle_set_creative_slot(
                &fixture.client.player,
                SSetCreativeSlot::new(36, reloaded.into()),
            )
            .unwrap();
    } else {
        assert!(
            fixture
                .client
                .player
                .inventory
                .get_stack(0)
                .are_equal(&before),
            "rejected B1 cannot change authoritative stock"
        );
    }
    let mut stack = ItemStack::new(1, &Item::DIAMOND);
    stack.patch.push((
        DataComponent::ItemName,
        Some(
            ItemNameImpl {
                name: "item.minecraft.diamond".into(),
            }
            .to_dyn(),
        ),
    ));
    fixture
        .client
        .client()
        .handle_set_creative_slot(
            &fixture.client.player,
            SSetCreativeSlot::new(36, stack.clone().into()),
        )
        .unwrap();
    let admitted = fixture.client.player.inventory.get_stack(0);
    let mut saved = NbtCompound::new();
    admitted.write_item_stack(&mut saved);
    let loaded = ItemStack::read_item_stack(&saved).unwrap();
    assert!(loaded.are_equal(&stack));
    assert_eq!(
        loaded.get_data_component::<ItemNameImpl>().unwrap().name,
        "item.minecraft.diamond"
    );
}
