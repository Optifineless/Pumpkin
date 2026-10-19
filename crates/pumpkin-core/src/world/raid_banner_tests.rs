use std::sync::Arc;

use crate::{
    entity::EntityBase, net::bedrock::combat_test_support::TestBedrockPlayer,
    net::java::combat_test_support::TestPlayer, server::combat_test_support, world::raid::Raid,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, CustomNameImpl, DataComponentImpl, EquipmentSlot,
        ItemNameImpl, Rarity, RarityImpl, TooltipDisplayImpl,
    },
    dye_color::DyeColor,
    item::Item,
    item_stack::ItemStack,
    packet::{CURRENT_MC_VERSION, clientbound::play::SET_EQUIPMENT},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::{
    ServerPacket,
    bedrock::{
        client::CMobArmorEquipment,
        network_item::{NetworkItemDescriptor, NetworkItemStackDescriptor},
    },
    codec::var_ulong::VarULong,
    java::client::play::CSetEquipment,
    ser::NetworkReadExt,
};
use pumpkin_util::{math::position::BlockPos, version::JavaMinecraftVersion};

fn expected_layers() -> Vec<BannerPatternLayer> {
    [
        ("minecraft:rhombus", DyeColor::Cyan),
        ("minecraft:stripe_bottom", DyeColor::LightGray),
        ("minecraft:stripe_center", DyeColor::Gray),
        ("minecraft:border", DyeColor::LightGray),
        ("minecraft:stripe_middle", DyeColor::Black),
        ("minecraft:half_horizontal", DyeColor::LightGray),
        ("minecraft:circle", DyeColor::LightGray),
        ("minecraft:border", DyeColor::Black),
    ]
    .into_iter()
    .map(|(pattern, color)| BannerPatternLayer {
        pattern: pattern.to_owned(),
        translation_key: None,
        color,
    })
    .collect()
}

fn assert_ominous_banner(stack: &ItemStack) {
    assert_eq!(stack.item.id, Item::WHITE_BANNER.id);
    assert_eq!(
        stack
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers,
        expected_layers()
    );
    assert_ominous_item_name(stack);
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(!tooltip.hide_tooltip);
    assert_eq!(
        tooltip
            .hidden_components
            .iter()
            .map(|component| component.to_name())
            .collect::<Vec<_>>(),
        vec![DataComponent::BannerPatterns.to_name()]
    );
    assert_eq!(
        stack.get_data_component::<RarityImpl>().unwrap().rarity,
        Rarity::Uncommon
    );
}

fn assert_ominous_item_name(stack: &ItemStack) {
    let actual = stack
        .get_data_component::<ItemNameImpl>()
        .unwrap()
        .write_data();
    let expected = ItemNameImpl::translated("block.minecraft.ominous_banner").write_data();
    assert_eq!(
        pumpkin_util::text::component_codec::canonical_component(&actual),
        pumpkin_util::text::component_codec::canonical_component(&expected)
    );
}

fn assert_ominous_banner_wire(stack: &ItemStack) {
    assert_eq!(stack.item.id, Item::WHITE_BANNER.id);
    let layers = &stack
        .get_data_component::<BannerPatternsImpl>()
        .unwrap()
        .layers;
    assert_eq!(
        layers
            .iter()
            .filter(|layer| layer.pattern != "murgicraft:unknown_pattern")
            .cloned()
            .collect::<Vec<_>>(),
        expected_layers()
    );
    assert_ominous_item_name(stack);
}

fn observer_equipment_packets(observer: &mut TestPlayer) -> Vec<CSetEquipment> {
    observer
        .take_packets()
        .into_iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            (data.get_var_int().ok()?.0 == SET_EQUIPMENT.to_id(CURRENT_MC_VERSION))
                .then(|| CSetEquipment::read(&mut data, &CURRENT_MC_VERSION).ok())
                .flatten()
        })
        .collect()
}

fn assert_legacy_captain_banner_loads_unchanged() {
    // Pre-fix captain saves used CustomName and had no BannerPatterns or ItemName patch.
    let mut legacy_stack = ItemStack::new(1, &Item::WHITE_BANNER);
    legacy_stack.set_data_component(CustomNameImpl {
        name: pumpkin_util::text::TextComponent::translate("block.minecraft.ominous_banner", []),
    });
    let mut legacy_nbt = NbtCompound::new();
    legacy_stack.write_item_stack(&mut legacy_nbt);
    let loaded_legacy = ItemStack::read_item_stack(&legacy_nbt).unwrap();
    assert!(loaded_legacy.patch.iter().all(|(component, _)| {
        *component != DataComponent::BannerPatterns && *component != DataComponent::ItemName
    }));
    assert!(
        loaded_legacy
            .get_data_component::<CustomNameImpl>()
            .is_some()
    );
    assert!(
        loaded_legacy
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .is_empty()
    );
}

fn assert_unknown_saved_pattern_is_preserved() {
    let unknown_pattern = BannerPatternLayer {
        pattern: "murgicraft:unknown_pattern".to_owned(),
        translation_key: None,
        color: DyeColor::Red,
    };
    let mut custom_stack = ItemStack::new(1, &Item::WHITE_BANNER);
    custom_stack.set_data_component(BannerPatternsImpl {
        layers: vec![unknown_pattern.clone()],
    });
    let mut custom_nbt = NbtCompound::new();
    custom_stack.write_item_stack(&mut custom_nbt);
    let loaded_custom = ItemStack::read_item_stack(&custom_nbt).unwrap();
    assert_eq!(
        loaded_custom
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers,
        vec![unknown_pattern]
    );
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep save/load and both observer edition tracker paths in one regression"
)]
async fn ominous_banner_raid_captain_components_survive_save_and_later_java_observation() {
    assert_eq!(CURRENT_MC_VERSION, JavaMinecraftVersion::V_26_3);
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    combat_test_support::publish_empty_chunk(
        &world,
        pumpkin_util::math::vector2::Vector2::new(0, 0),
    );

    let mut raid = Raid::new(
        1,
        BlockPos::new(8, 64, 8),
        world.level_info.load().difficulty,
    );
    raid.spawn_group(&world, BlockPos::new(8, 64, 8));

    let captain_uuid = *raid.group_to_leader_map.get(&1).unwrap();
    let captain = world
        .entities
        .load()
        .iter()
        .find(|entity| entity.get_entity().entity_uuid == captain_uuid)
        .unwrap()
        .clone();
    let mob = captain.get_mob().unwrap();
    let raider = mob.as_raider().unwrap();
    assert!(raider.is_patrol_leader(), "raid captain leadership changed");
    let living = captain.get_living_entity().unwrap();
    let stack = living
        .entity_equipment
        .try_lock()
        .unwrap()
        .get(&EquipmentSlot::HEAD);
    assert_ominous_banner(&stack);

    let mut saved_stack = NbtCompound::new();
    stack.write_item_stack(&mut saved_stack);
    let loaded_stack = ItemStack::read_item_stack(&saved_stack).unwrap();
    assert_ominous_banner(&loaded_stack);

    assert_legacy_captain_banner_loads_unchanged();
    assert_unknown_saved_pattern_is_preserved();

    // Keep an unknown datapack pattern in the equipped stack. Its Java wire layer
    // is filtered independently, so pairing still publishes the chest slot too.
    let unknown_layer = BannerPatternLayer {
        pattern: "murgicraft:unknown_pattern".to_owned(),
        translation_key: None,
        color: DyeColor::Red,
    };
    let mut equipped_head = stack.clone();
    let mut equipped_patterns = equipped_head
        .get_data_component::<BannerPatternsImpl>()
        .unwrap()
        .clone();
    equipped_patterns.layers.push(unknown_layer.clone());
    equipped_head.set_data_component(equipped_patterns);
    living
        .entity_equipment
        .lock()
        .unwrap()
        .equipment
        .insert(EquipmentSlot::HEAD, equipped_head.clone());
    living.entity_equipment.lock().unwrap().equipment.insert(
        EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::IRON_CHESTPLATE),
    );
    let mut unknown_saved = NbtCompound::new();
    equipped_head.write_item_stack(&mut unknown_saved);
    assert_eq!(
        ItemStack::read_item_stack(&unknown_saved)
            .unwrap()
            .get_data_component::<BannerPatternsImpl>()
            .unwrap()
            .layers
            .last(),
        Some(&unknown_layer)
    );

    // Create a later observer after the captain exists to exercise tracker pairing publication.
    let mut observer = TestPlayer::new(&world);
    observer
        .player
        .get_entity()
        .set_pos(pumpkin_util::math::vector3::Vector3::new(8.5, 64.0, 8.5));
    observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(pumpkin_util::math::vector2::Vector2::new(0, 0));
    world.players.store(Arc::new(vec![observer.player.clone()]));
    let tracked = world
        .entity_tracker
        .get_tracked_entity(captain.get_entity().entity_id)
        .unwrap();
    tracked.seen_by.insert(observer.player.gameprofile.id);
    tracked.add_pairing(&observer.player);

    let equipment_packet = observer_equipment_packets(&mut observer)
        .into_iter()
        .filter(|packet| packet.entity_id.0 == captain.get_entity().entity_id)
        .find(|packet| {
            packet
                .equipment
                .iter()
                .any(|(slot, _)| *slot == EquipmentSlot::HEAD.discriminant())
                && packet
                    .equipment
                    .iter()
                    .any(|(slot, _)| *slot == EquipmentSlot::CHEST.discriminant())
        })
        .expect("pairing omitted head or unrelated chest equipment");
    let head = equipment_packet
        .equipment
        .into_iter()
        .find(|(slot, _)| *slot == EquipmentSlot::HEAD.discriminant())
        .map(|(_, stack)| stack.0.into_owned())
        .expect("later observer received no captain head equipment");
    assert_ominous_banner_wire(&head);

    // Bedrock equipment updates use NetworkItemStackDescriptor's User Data NBT.
    // Exercise the same living-equipment publication path used by armor changes.
    let mut bedrock_observer = TestBedrockPlayer::new(&world).await;
    bedrock_observer
        .player
        .get_entity()
        .set_pos(pumpkin_util::math::vector3::Vector3::new(8.5, 64.0, 8.5));
    bedrock_observer
        .player
        .watched_section
        .store(observer.player.watched_section.load());
    bedrock_observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(pumpkin_util::math::vector2::Vector2::new(0, 0));
    let tracked = world
        .entity_tracker
        .get_tracked_entity(captain.get_entity().entity_id)
        .unwrap();
    tracked.update_player(&bedrock_observer.player, &world);
    assert!(
        tracked
            .seen_by
            .contains(&bedrock_observer.player.gameprofile.id)
    );
    living.send_equipment_changes(&[(EquipmentSlot::HEAD, equipped_head)]);

    let bedrock_head = living
        .entity_equipment
        .lock()
        .unwrap()
        .get(&EquipmentSlot::HEAD);
    let bedrock_chest = living
        .entity_equipment
        .lock()
        .unwrap()
        .get(&EquipmentSlot::CHEST);
    let descriptor = NetworkItemStackDescriptor::from(&bedrock_head);
    assert!(
        descriptor
            .extra_data
            .windows(8)
            .any(|bytes| bytes == b"Patterns")
    );
    assert!(descriptor.extra_data.windows(2).any(|bytes| bytes == b"mr"));
    let descriptor_nbt = NetworkItemDescriptor::from(&bedrock_head).nbt_data;
    assert_eq!(descriptor_nbt.root_tag.get_int("Type"), Some(0));
    let Some(NbtTag::List(bedrock_layers)) = descriptor_nbt.root_tag.get("Patterns") else {
        panic!("Bedrock captain banner omitted Patterns NBT");
    };
    assert_eq!(bedrock_layers.len(), 8);

    let armor_packet = CMobArmorEquipment {
        target_runtime_id: VarULong(captain.get_entity().entity_id as u64),
        head: descriptor,
        torso: NetworkItemStackDescriptor::from(&bedrock_chest),
        legs: NetworkItemStackDescriptor::default(),
        feet: NetworkItemStackDescriptor::default(),
        body: NetworkItemStackDescriptor::default(),
    };
    let expected_packet = bedrock_observer
        .client()
        .serialize_packet(&armor_packet)
        .unwrap();
    let received_packets = bedrock_observer.take_packets();
    assert!(
        received_packets.iter().any(|packet| {
            packet.starts_with(&expected_packet[..4])
                && packet.windows(8).any(|bytes| bytes == b"Patterns")
                && packet.windows(2).any(|bytes| bytes == b"mr")
        }),
        "Bedrock observer did not receive banner patterns in armor equipment: {received_packets:?}"
    );

    world.level.shutdown().await.unwrap();
}
