use crate::{
    entity::player::Player,
    world::map::{HALF_MAP_SIZE, MAP_SIZE, MAX_SCALE, MapData},
};
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{MapIdImpl, MapPostProcessingImpl},
    item_stack::ItemStack,
};
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[path = "map_crafting_click_tests.rs"]
mod click_tests;

/// Looks up the map that cartography operations will copy.
pub fn saved_state(player: &Player, stack: &ItemStack) -> Option<(i8, bool)> {
    let server = player.world().server.upgrade()?;
    let map = server
        .map_manager
        .get_map(stack.get_data_component::<MapIdImpl>()?.id)?;
    let map = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some((map.scale, map.locked))
}

/// Consumes the processing component and allocates the scaled or locked map id.
pub fn on_crafted_post_process(player: &Player, stack: &mut ItemStack) {
    // MapItem.onCraftedPostProcess -> scaleMap / lockMap.
    let Some(processing) = stack.get_data_component::<MapPostProcessingImpl>().copied() else {
        return;
    };
    stack.remove_data_component(DataComponent::MapPostProcessing);
    let Some(server) = player.world().server.upgrade() else {
        return;
    };
    let Some(id) = stack.get_data_component::<MapIdImpl>() else {
        return;
    };
    let Some(original) = server.map_manager.get_map(id.id) else {
        return;
    };
    let original = original
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let copy = processed_copy(&original, processing);
    drop(original);
    let id = server.next_map_id();
    server
        .map_manager
        .maps
        .insert(id, Arc::new(Mutex::new(copy)));
    stack.set_data_component(MapIdImpl { id });
}

fn processed_copy(original: &MapData, processing: MapPostProcessingImpl) -> MapData {
    // MapItemSavedData.scaled / locked: scaling starts a new map; locking copies pixels/decorations.
    if processing == MapPostProcessingImpl::SCALE {
        let scale = (original.scale + 1).clamp(0, MAX_SCALE);
        let size = MAP_SIZE * (1 << scale);
        let center = |v: i32| {
            ((f64::from(v) + f64::from(HALF_MAP_SIZE)) / f64::from(size)).floor() as i32 * size
                + size / 2
                - HALF_MAP_SIZE
        };
        let mut copy = MapData::new(
            original.dimension.clone(),
            center(original.center_x),
            center(original.center_z),
            scale,
        );
        copy.tracking_position = original.tracking_position;
        copy.unlimited_tracking = original.unlimited_tracking;
        copy
    } else {
        let mut copy = MapData::new(
            original.dimension.clone(),
            original.center_x,
            original.center_z,
            original.scale,
        );
        copy.locked = true;
        copy.tracking_position = original.tracking_position;
        copy.unlimited_tracking = original.unlimited_tracking;
        copy.banners.clone_from(&original.banners);
        copy.colors.clone_from(&original.colors);
        copy.decorations.clone_from(&original.decorations);
        copy.fully_updated = true;
        copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::dimension::Dimension;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deep_review_scaled_and_locked_maps_resolve_after_save_and_reload() {
        use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
        use pumpkin_data::item::Item;
        use pumpkin_nbt::compound::NbtCompound;
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let original = server
            .map_manager
            .create_map(47, Dimension::OVERWORLD, -64, 192, 0);
        original.lock().unwrap().colors[0] = 231;
        let mut results = Vec::new();
        for processing in [MapPostProcessingImpl::SCALE, MapPostProcessingImpl::LOCK] {
            let mut stack = ItemStack::new(1, &Item::FILLED_MAP);
            stack.set_data_component(MapIdImpl { id: 47 });
            stack.set_data_component(processing);
            on_crafted_post_process(&fixture.player, &mut stack);
            let mut saved_item = NbtCompound::new();
            stack.write_item_stack(&mut saved_item);
            results.push(ItemStack::read_item_stack(&saved_item).unwrap());
        }
        server
            .map_manager
            .save(dir.path(), server.level_info.load().data_version)
            .await
            .unwrap();
        let locked_id = results[1].get_data_component::<MapIdImpl>().unwrap().id;
        let saved_path = dir
            .path()
            .join(format!("data/minecraft/maps/{locked_id}.dat"));
        let mut input = flate2::read::GzDecoder::new(std::fs::File::open(saved_path).unwrap());
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut input, &mut bytes).unwrap();
        let document =
            pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
                &mut std::io::Cursor::new(bytes),
            ))
            .unwrap();
        let data = document.root_tag.get_compound("data").unwrap();
        assert_eq!(data.get_byte("scale"), Some(0));
        assert_eq!(data.get_int("xCenter"), Some(-64));
        assert_eq!(data.get_bool("locked"), Some(true));
        assert_eq!(data.get_byte_array("colors").unwrap()[0] as u8, 231);
        // Reload the backing records, then resolve the persisted item IDs through the cache.
        server.map_manager.maps.clear();
        let reloaded = crate::world::map::MapManager::load(dir.path()).unwrap();
        for result in &results {
            reloaded
                .load_map(result.get_data_component::<MapIdImpl>().unwrap().id)
                .await
                .unwrap();
        }
        for entry in reloaded.maps.iter() {
            server
                .map_manager
                .maps
                .insert(*entry.key(), entry.value().clone());
        }
        assert_eq!(saved_state(&fixture.player, &results[0]), Some((1, false)));
        assert_eq!(saved_state(&fixture.player, &results[1]), Some((0, true)));
        let scaled = server
            .map_manager
            .get_map(results[0].get_data_component::<MapIdImpl>().unwrap().id)
            .unwrap();
        let scaled = {
            let map = scaled.lock().unwrap();
            (map.center_x, map.center_z, map.colors[0])
        };
        assert_eq!(scaled, (64, 320, 0));
        let locked = server
            .map_manager
            .get_map(results[1].get_data_component::<MapIdImpl>().unwrap().id)
            .unwrap();
        let locked = {
            let map = locked.lock().unwrap();
            (map.center_x, map.center_z, map.colors[0])
        };
        assert_eq!(locked, (-64, 192, 231));
        assert!(world.level.shutdown().await.is_ok());
        crate::server::fixture_lifecycle::finish().await;
    }

    #[test]
    fn scaling_realigns_center_and_starts_with_empty_pixels() {
        let mut original = MapData::new(Dimension::OVERWORLD, -64, 192, 0);
        original.colors[0] = 12;
        let scaled = processed_copy(&original, MapPostProcessingImpl::SCALE);
        assert_eq!(
            (scaled.scale, scaled.center_x, scaled.center_z),
            (1, 64, 320)
        );
        assert_eq!(scaled.colors[0], 0);
        assert_eq!(original.colors[0], 12);
    }

    #[test]
    fn locking_copies_pixels_and_keeps_original_unlocked() {
        let mut original = MapData::new(Dimension::OVERWORLD, 64, -192, 2);
        original.colors[9] = 37;
        let locked = processed_copy(&original, MapPostProcessingImpl::LOCK);
        assert!(locked.locked);
        assert_eq!(
            (locked.scale, locked.center_x, locked.center_z),
            (2, 64, -192)
        );
        assert_eq!(locked.colors[9], 37);
        assert!(!original.locked);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn taking_cartography_result_allocates_processed_map() {
        use crate::{net::java::combat_test_support::TestPlayer, server::combat_test_support};
        use pumpkin_data::item::Item;
        use pumpkin_inventory::{
            Inventory, cartography_table_screen_handler::CartographyTableScreenHandler,
            screen_handler::ScreenHandler,
        };
        use pumpkin_protocol::java::server::play::SlotActionType;
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = combat_test_support::world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let original = server
            .map_manager
            .create_map(47, Dimension::OVERWORLD, 0, 0, 0);
        original.lock().unwrap().colors[0] = 37;
        let mut map = ItemStack::new(1, &Item::FILLED_MAP);
        map.set_data_component(MapIdImpl { id: 47 });
        let mut menu = CartographyTableScreenHandler::new(1, &player.inventory);
        menu.input_inventory.set_stack(0, map);
        menu.input_inventory
            .set_stack(1, ItemStack::new(1, &Item::GLASS_PANE));
        menu.slots_changed(player.as_ref());
        menu.on_slot_click(2, 0, SlotActionType::Pickup, player.as_ref());
        let result = menu.get_behaviour().cursor_stack.lock().unwrap().clone();
        assert!(
            result
                .get_data_component::<MapPostProcessingImpl>()
                .is_none()
        );
        let id = result.get_data_component::<MapIdImpl>().unwrap().id;
        assert_ne!(id, 47);
        let locked = server.map_manager.get_map(id).unwrap();
        assert!(locked.lock().unwrap().locked);
        locked.lock().unwrap().update(player);
        assert_eq!(locked.lock().unwrap().colors[0], 37);
        assert!(!original.lock().unwrap().locked);
        assert!(world.level.shutdown().await.is_ok());
        crate::server::fixture_lifecycle::finish().await;
    }
}
