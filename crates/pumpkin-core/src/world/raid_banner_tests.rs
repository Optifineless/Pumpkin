use std::sync::Arc;

use crate::{
    entity::EntityBase, net::java::combat_test_support::TestPlayer, server::combat_test_support,
    world::raid::Raid,
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, CustomNameImpl, EquipmentSlot, ItemNameImpl,
        Rarity, RarityImpl, TooltipDisplayImpl,
    },
    dye_color::DyeColor,
    item::Item,
    item_stack::ItemStack,
    packet::{CURRENT_MC_VERSION, clientbound::play::SET_EQUIPMENT},
};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{ServerPacket, java::client::play::CSetEquipment, ser::NetworkReadExt};
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
    assert_eq!(
        stack
            .get_data_component::<ItemNameImpl>()
            .unwrap()
            .name
            .as_ref(),
        "block.minecraft.ominous_banner"
    );
    let tooltip = stack.get_data_component::<TooltipDisplayImpl>().unwrap();
    assert!(!tooltip.hide_tooltip);
    assert!(!tooltip.hide_additional_tooltip);
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

    let head = observer_equipment_packets(&mut observer)
        .into_iter()
        .filter(|packet| packet.entity_id.0 == captain.get_entity().entity_id)
        .flat_map(|packet| packet.equipment)
        .find(|(slot, _)| *slot == EquipmentSlot::HEAD.discriminant())
        .map(|(_, stack)| stack.0.into_owned())
        .expect("later observer received no captain head equipment");
    assert_ominous_banner(&head);

    world.level.shutdown().await.unwrap();
}
