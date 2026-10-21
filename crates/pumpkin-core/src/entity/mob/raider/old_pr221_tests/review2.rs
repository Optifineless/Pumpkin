use super::support::{Fixture, Item, ItemStack, NbtCompound, equipment, read_published_stack};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternsImpl, EquipmentSlot, Rarity, RarityImpl, TooltipDisplayImpl,
    },
    dye_color::DyeColor,
    packet::{CURRENT_MC_VERSION, clientbound::play::CONTAINER_SET_CONTENT},
};
use pumpkin_inventory::{
    Inventory, SimpleInventory,
    generic_container_screen_handler::create_generic_9x3,
    screen_handler::{ScreenMutex, SharedScreenHandler},
};
use pumpkin_nbt::{Nbt, deserializer::NbtReadHelperJava, tag::NbtTag};
use pumpkin_protocol::{
    ServerPacket, VarInt,
    java::server::play::SSetCreativeSlot,
    ser::{NetworkReadExt, NetworkWriteExt},
};
use pumpkin_util::{GameMode, text::TextComponent};
use std::sync::Arc;

fn disk_round_trip(tag: NbtCompound) -> NbtCompound {
    let bytes = Nbt::new(String::new(), tag).write();
    Nbt::read(&mut NbtReadHelperJava::new(std::io::Cursor::new(
        bytes.as_ref(),
    )))
    .unwrap()
    .root_tag
}

fn saved_inventory(item_type: &Item, component: &str, value: NbtTag) -> NbtCompound {
    let mut components = NbtCompound::new();
    components.put(component, value);
    components.put_string("minecraft:rarity", "uncommon".into());
    let mut item = NbtCompound::new();
    item.put_string("id", format!("minecraft:{}", item_type.registry_key));
    item.put_int("count", 1);
    item.put_byte("Slot", 0);
    item.put_compound("components", components);
    let mut unrelated = NbtCompound::new();
    unrelated.put_string("id", "minecraft:diamond".into());
    unrelated.put_int("count", 7);
    unrelated.put_byte("Slot", 1);
    let mut inventory = NbtCompound::new();
    inventory.put_list(
        "Items",
        vec![NbtTag::Compound(item), NbtTag::Compound(unrelated)],
    );
    disk_round_trip(inventory)
}

fn load_inventory(nbt: &NbtCompound) -> Arc<SimpleInventory> {
    let inventory = Arc::new(SimpleInventory::new(27));
    inventory.read_data(nbt, &mut inventory.stacks.write().unwrap());
    inventory
}

fn reload_inventory(inventory: &SimpleInventory) -> Arc<SimpleInventory> {
    let mut saved = NbtCompound::new();
    inventory.write_inventory_nbt(&mut saved, true);
    load_inventory(&disk_round_trip(saved))
}

fn assert_preserved(inventory: &SimpleInventory, item: &Item) -> ItemStack {
    let stack = inventory.get_stack(0);
    assert_eq!(stack.item_count, 1, "saved item must retain its item count");
    assert_eq!(stack.item.id, item.id);
    assert_eq!(
        stack.get_data_component::<RarityImpl>().unwrap().rarity,
        Rarity::Uncommon
    );
    assert!(
        inventory
            .get_stack(1)
            .are_equal(&ItemStack::new(7, &Item::DIAMOND))
    );
    stack
}

#[test]
fn old_pr221_review2_saved_1025_banner_layers_survive_inventory_reload() {
    let mut layer = NbtCompound::new();
    layer.put_string("pattern", "minecraft:border".into());
    layer.put_string("color", "black".into());
    let saved = saved_inventory(
        &Item::WHITE_BANNER,
        "minecraft:banner_patterns",
        NbtTag::List(vec![NbtTag::Compound(layer); 1025]),
    );
    let inventory = load_inventory(&saved);
    for inventory in [inventory.clone(), reload_inventory(&inventory)] {
        let stack = assert_preserved(&inventory, &Item::WHITE_BANNER);
        let patterns = stack.get_data_component::<BannerPatternsImpl>().unwrap();
        assert_eq!(patterns.layers.len(), 1025, "no saved layer may disappear");
        for layer in &patterns.layers {
            assert_eq!(layer.pattern, "minecraft:border");
            assert_eq!(layer.color, DyeColor::Black);
            assert_eq!(layer.translation_key, None);
        }
        // BannerPatternLayers.STREAM_CODEC has no layer truncation either.
        let mut encoded = Vec::new();
        pumpkin_protocol::codec::data_component::serialize(
            DataComponent::BannerPatterns,
            patterns,
            &mut encoded,
        )
        .unwrap();
        let mut encoded = encoded.as_slice();
        assert_eq!(encoded.get_var_int().unwrap().0, 1025);
        assert_eq!(encoded, [2, 15].repeat(1025));
    }
}

#[test]
fn old_pr221_review2_legacy_banner_inputs_keep_their_saved_meaning() {
    for value in [NbtTag::Int(1), NbtTag::List(vec![NbtTag::Int(1)])] {
        let inventory = load_inventory(&saved_inventory(
            &Item::WHITE_BANNER,
            "minecraft:banner_patterns",
            value,
        ));
        for inventory in [inventory.clone(), reload_inventory(&inventory)] {
            assert!(
                assert_preserved(&inventory, &Item::WHITE_BANNER)
                    .get_data_component::<BannerPatternsImpl>()
                    .unwrap()
                    .layers
                    .is_empty()
            );
        }
    }
    let pattern = "x".repeat(40000);
    let mut layer = NbtCompound::new();
    layer.put_string("pattern", pattern.clone());
    layer.put_string("color", "unknown".into());
    let inventory = load_inventory(&saved_inventory(
        &Item::WHITE_BANNER,
        "minecraft:banner_patterns",
        NbtTag::List(vec![NbtTag::Compound(layer)]),
    ));
    for inventory in [inventory.clone(), reload_inventory(&inventory)] {
        let stack = assert_preserved(&inventory, &Item::WHITE_BANNER);
        let layers = &stack
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].pattern, pattern);
        assert_eq!(layers[0].color, DyeColor::Black);
    }
}

#[test]
fn old_pr221_review2_saved_duplicate_tooltip_names_survive_inventory_reload() {
    let mut tooltip = NbtCompound::new();
    tooltip.put_list(
        "hidden_components",
        vec![NbtTag::String("minecraft:banner_patterns".into()); 257],
    );
    let inventory = load_inventory(&saved_inventory(
        &Item::STONE,
        "minecraft:tooltip_display",
        NbtTag::Compound(tooltip),
    ));
    for inventory in [inventory.clone(), reload_inventory(&inventory)] {
        let stack = assert_preserved(&inventory, &Item::STONE);
        let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
        assert!(!tooltip.hide_tooltip);
        assert!(tooltip.hidden_components == [DataComponent::BannerPatterns]);
    }
}

#[test]
fn old_pr221_review2_saved_numeric_tooltip_bools_survive_inventory_reload() {
    for (value, expected) in [
        (NbtTag::Int(0), false),
        (NbtTag::Double(-0.5), true),
        (NbtTag::Byte(1), true),
        (NbtTag::Short(-1), true),
        (NbtTag::Long(0), false),
        (NbtTag::Float(0.25), true),
    ] {
        let mut tooltip = NbtCompound::new();
        tooltip.put("hide_tooltip", value);
        let inventory = load_inventory(&saved_inventory(
            &Item::STONE,
            "minecraft:tooltip_display",
            NbtTag::Compound(tooltip),
        ));
        for inventory in [inventory.clone(), reload_inventory(&inventory)] {
            let stack = assert_preserved(&inventory, &Item::STONE);
            assert_eq!(
                stack
                    .get_data_component::<TooltipDisplayImpl>()
                    .unwrap()
                    .hide_tooltip,
                expected
            );
        }
    }
}

#[test]
fn old_pr221_review2_malformed_tooltip_does_not_delete_saved_inventory_item() {
    let mut tooltip = NbtCompound::new();
    tooltip.put_string("hide_tooltip", "invalid".into());
    assert!(TooltipDisplayImpl::read_data(&NbtTag::Compound(tooltip.clone())).is_none());
    let inventory = load_inventory(&saved_inventory(
        &Item::STONE,
        "minecraft:tooltip_display",
        NbtTag::Compound(tooltip),
    ));
    for inventory in [inventory.clone(), reload_inventory(&inventory)] {
        let stack = assert_preserved(&inventory, &Item::STONE);
        assert!(
            !stack
                .patch
                .iter()
                .any(|(id, _)| *id == DataComponent::TooltipDisplay)
        );
    }
}

fn admitted_multibyte_banner(fixture: &Fixture) -> ItemStack {
    fixture.client.player.set_gamemode(GameMode::Creative);
    // Independent wire fixture: one direct holder, asset minecraft:foo, 16384 UTF-16 units.
    let mut payload = vec![1, 0, 13];
    payload.extend_from_slice(b"minecraft:foo");
    payload.extend_from_slice(&[0x80, 0x80, 0x02]); // 32768 UTF-8 bytes
    payload.extend_from_slice("é".repeat(16384).as_bytes());
    payload.push(9); // cyan
    let mut bytes = vec![0, 36, 1, 0x89, 0x0b, 1, 0, 74];
    bytes.write_var_int(&VarInt(payload.len() as i32)).unwrap();
    bytes.extend_from_slice(&payload);
    let mut raw = bytes.as_slice();
    let packet = SSetCreativeSlot::read_bounded(&mut raw, &CURRENT_MC_VERSION).unwrap();
    assert!(raw.is_empty());
    fixture
        .client
        .client()
        .handle_set_creative_slot(&fixture.client.player, packet)
        .unwrap();
    let admitted = fixture.client.player.inventory.get_stack(0);
    assert_eq!(
        admitted.item_count, 1,
        "real Creative handler admitted the banner"
    );
    let inventory = SimpleInventory::new(27);
    inventory.set_stack(0, admitted.clone());
    inventory.set_stack(1, ItemStack::new(7, &Item::DIAMOND));
    let loaded = reload_inventory(&inventory);
    assert!(
        loaded
            .get_stack(1)
            .are_equal(&ItemStack::new(7, &Item::DIAMOND))
    );
    let stack = loaded.get_stack(0);
    assert!(
        stack.are_equal(&admitted),
        "actual NBT bytes preserve the admitted direct holder"
    );
    let patterns = stack.get_data_component::<BannerPatternsImpl>().unwrap();
    assert_eq!(
        patterns.layers[0].translation_key.as_deref(),
        Some("é".repeat(16384).as_str())
    );
    assert_eq!(patterns.layers[0].pattern, "minecraft:foo");
    assert_eq!(patterns.layers[0].color, DyeColor::Cyan);
    stack
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_review2_multibyte_direct_holder_keeps_full_container_send() {
    Fixture::new()
        .verify(|fixture| {
            let stack = admitted_multibyte_banner(fixture);
            let inventory = Arc::new(SimpleInventory::new(27));
            inventory.set_stack(0, stack.clone());
            inventory.set_stack(1, ItemStack::new(7, &Item::DIAMOND));
            let screen: SharedScreenHandler = Arc::new(ScreenMutex::new(create_generic_9x3(
                1,
                &fixture.client.player.inventory,
                inventory,
                fixture.client.player.as_ref(),
            )));
            fixture.client.take_packets();
            fixture
                .client
                .player
                .open_handled_screen_direct(screen, &TextComponent::text("Boundary chest"));
            let packets = fixture.client.take_packets();
            let content = packets
                .iter()
                .find(|bytes| bytes.as_ref().get_var_int().unwrap().0 == CONTAINER_SET_CONTENT.0);
            assert!(
                content.is_some(),
                "opening chest must send its full contents, including unrelated stock"
            );
            let mut raw = content.unwrap().as_ref();
            raw.get_var_int().unwrap();
            assert_eq!(raw.get_container_id(&CURRENT_MC_VERSION).unwrap().0, 1);
            raw.get_var_int().unwrap();
            let count = raw.get_var_int().unwrap().0;
            assert_eq!(count, 63);
            let slots: Vec<_> = (0..count).map(|_| read_published_stack(&mut raw)).collect();
            assert!(slots[0].are_equal(&stack));
            assert!(slots[1].are_equal(&ItemStack::new(7, &Item::DIAMOND)));
            assert!(read_published_stack(&mut raw).is_empty());
            assert!(raw.is_empty(), "complete full-container packet consumption");
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_review2_multibyte_direct_holder_keeps_full_equipment_send() {
    Fixture::new()
        .verify(|fixture| {
            let stack = admitted_multibyte_banner(fixture);
            let mob = fixture.pillager();
            let armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
            {
                let mut equipment = mob
                    .get_living_entity()
                    .unwrap()
                    .entity_equipment
                    .lock()
                    .unwrap();
                equipment.put(&EquipmentSlot::HEAD, stack.clone());
                equipment.put(&EquipmentSlot::CHEST, armor.clone())
            };
            let tracked = fixture
                .world
                .entity_tracker
                .get_tracked_entity(mob.get_entity().entity_id)
                .unwrap();
            fixture.client.take_packets();
            tracked.add_pairing(&fixture.client.player);
            let sent = equipment(&mut fixture.client, mob.get_entity().entity_id);
            assert!(
                sent.contains_key(&EquipmentSlot::HEAD.discriminant()),
                "full equipment send must reach observer"
            );
            assert!(sent[&EquipmentSlot::HEAD.discriminant()].are_equal(&stack));
            assert!(sent[&EquipmentSlot::CHEST.discriminant()].are_equal(&armor));
        })
        .await;
}
