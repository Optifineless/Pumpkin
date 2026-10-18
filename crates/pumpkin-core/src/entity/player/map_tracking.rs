use crate::{entity::player::Player, server::Server};
use pumpkin_data::{data_component_impl::MapIdImpl, item::Item};
use pumpkin_protocol::{
    VarInt,
    java::client::play::{CMapItemData, MapIcon, MapPatch},
};
use pumpkin_util::Hand;
use std::sync::atomic::Ordering;

use crate::world::map::MapDecoration;

struct MapPacketSnapshot {
    scale: i8,
    locked: bool,
    center_x: i32,
    center_z: i32,
    include_player_marker: bool,
    decorations: Vec<MapDecoration>,
    colors: Option<Box<[u8; 128 * 128]>>,
}

pub(super) fn tick_maps(player: &Player, server: &Server) {
    let tick_count = player.tick_counter.load(Ordering::Relaxed);
    let player_dimension = player.world().dimension.minecraft_name;

    for hand in Hand::all() {
        let stack = player.inventory().get_stack_in_hand(hand);
        if stack.item.id != Item::FILLED_MAP.id {
            continue;
        }
        let Some(map_id) = stack.get_data_component::<MapIdImpl>().map(|map| map.id) else {
            continue;
        };
        let Some(map_data_arc) = server.map_manager.get_map(map_id) else {
            continue;
        };

        let snapshot = {
            let Ok(mut map_data) = map_data_arc.try_lock() else {
                continue;
            };
            map_data.update(player);
            if !map_data.dirty && tick_count % 10 != 0 {
                continue;
            }

            let snapshot = MapPacketSnapshot {
                scale: map_data.scale,
                locked: map_data.locked,
                center_x: map_data.center_x,
                center_z: map_data.center_z,
                include_player_marker: map_data.dimension.minecraft_name == player_dimension,
                decorations: map_data.decorations.clone(),
                colors: map_data.dirty.then(|| map_data.colors.clone()),
            };
            map_data.dirty = false;
            snapshot
        };

        // MapItemSavedData.tickCarriedBy keeps map decorations active independently of raster updates.
        let mut icons = Vec::with_capacity(
            snapshot.decorations.len() + usize::from(snapshot.include_player_marker),
        );
        if snapshot.include_player_marker {
            let scale = 1 << snapshot.scale;
            let position = player.position();
            let dx = position.x - f64::from(snapshot.center_x);
            let dz = position.z - f64::from(snapshot.center_z);
            let raw_x = dx / f64::from(scale) * 2.0;
            let raw_z = dz / f64::from(scale) * 2.0;
            let is_off_map =
                !(-127.0..=127.0).contains(&raw_x) || !(-127.0..=127.0).contains(&raw_z);
            let icon_x = raw_x.clamp(-128.0, 127.0) as i8;
            let icon_z = raw_z.clamp(-128.0, 127.0) as i8;
            let yaw = player.living_entity.entity.yaw.load();
            let icon_direction = ((((yaw * 16.0 / 360.0).round() as i32 + 8) % 16 + 16) % 16) as i8;
            let decoration_type = if is_off_map {
                &pumpkin_data::map_decoration::MapDecorationType::PLAYER_OFF_MAP
            } else {
                &pumpkin_data::map_decoration::MapDecorationType::PLAYER
            };
            icons.push(MapIcon {
                icon_type: VarInt(decoration_type.id as i32),
                x: icon_x,
                z: icon_z,
                direction: icon_direction,
                display_name: None,
            });
        }
        icons.extend(snapshot.decorations.into_iter().map(|decoration| MapIcon {
            icon_type: VarInt(decoration.icon_type),
            x: decoration.x,
            z: decoration.z,
            direction: decoration.direction,
            display_name: decoration.display_name,
        }));

        let data = snapshot.colors.as_ref().map(|colors| MapPatch {
            columns: 128,
            rows: 128,
            x: 0,
            z: 0,
            data: &colors[..],
        });
        player.try_send_client_packet(&CMapItemData {
            map_id: VarInt(map_id),
            scale: snapshot.scale,
            tracking_position: true,
            locked: snapshot.locked,
            icons: Some(&icons),
            data,
        });
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test fixture construction must succeed")]
mod tests {
    use super::*;
    use crate::{
        entity::{EntityBase, death_test_world::DeathTestWorld},
        net::java::combat_test_support::TestPlayer,
        world::map::MapData,
    };
    use pumpkin_data::{
        data_component::DataComponent, data_component_impl::DataComponentImpl,
        dimension::Dimension, item_stack::ItemStack, map_decoration::MapDecorationType,
    };
    use pumpkin_protocol::java::client::play::CMapItemData;
    use pumpkin_util::math::vector3::Vector3;

    fn filled_map(map_id: i32) -> ItemStack {
        let mut stack = ItemStack::new(1, &Item::FILLED_MAP);
        stack.patch.push((
            DataComponent::MapId,
            Some(MapIdImpl { id: map_id }.to_dyn()),
        ));
        stack
    }

    fn prepare_map(
        server: &crate::server::Server,
        dimension: Dimension,
        color: u8,
        locked: bool,
    ) -> (i32, std::sync::Arc<std::sync::Mutex<MapData>>) {
        let map_id = server.next_map_id();
        let map = server.map_manager.create_map(map_id, dimension, 0, 0, 0);
        {
            let mut map_data = map.lock().unwrap();
            map_data.colors[0] = color;
            map_data.dirty = false;
            map_data.locked = locked;
        };
        (map_id, map)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tick_maps_preserves_cross_dimension_pixels_and_carried_map_packets() {
        let fixture = DeathTestWorld::new().await;
        let server = fixture.server.clone();
        let nether = server.get_world_from_dimension(&Dimension::THE_NETHER);
        let mut client = TestPlayer::new(&nether);
        let player = client.player.clone();
        player.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));

        let (map_id, map) = prepare_map(&server, Dimension::OVERWORLD, 173, false);
        {
            let mut map_data = map.lock().unwrap();
            map_data.decorations.push(MapDecoration {
                icon_type: MapDecorationType::BANNER_RED.id as i32,
                x: 7,
                z: -9,
                direction: 4,
                display_name: None,
            });
        };
        let before = map.lock().unwrap().colors.clone();
        let stack = filled_map(map_id);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, stack.clone());
        player.inventory().set_stack_in_hand(Hand::Left, stack);
        player.tick_counter.store(10, Ordering::Relaxed);
        client.take_packets();

        player.tick_maps(&server);

        {
            let map_data = map.lock().unwrap();
            assert!(
                map_data.colors.as_slice() == before.as_slice(),
                "wrong-dimension sampling changed the full pixel array; first pixel is {}",
                map_data.colors[0]
            );
            assert!(!map_data.dirty);
            assert!(!map_data.fully_updated);
            assert_eq!(map_data.decorations.len(), 1);
        };

        let actual_packets = client.take_packets();
        let expected_icons = [MapIcon {
            icon_type: VarInt(MapDecorationType::BANNER_RED.id as i32),
            x: 7,
            z: -9,
            direction: 4,
            display_name: None,
        }];
        let expected_packet = CMapItemData {
            map_id: VarInt(map_id),
            scale: 0,
            tracking_position: true,
            locked: false,
            icons: Some(&expected_icons),
            data: None,
        };
        let expected_bytes = client.client().serialize_packet(&expected_packet).unwrap();
        assert_eq!(actual_packets, vec![expected_bytes.clone(), expected_bytes]);

        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tick_maps_keeps_same_dimension_updates_and_locked_map_pixels() {
        let fixture = DeathTestWorld::new().await;
        let server = fixture.server.clone();
        let overworld = server.get_world_from_dimension(&Dimension::OVERWORLD);
        let client = TestPlayer::new(&overworld);
        let player = client.player.clone();
        player.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));

        let (updating_id, updating_map) = prepare_map(&server, Dimension::OVERWORLD, 173, false);
        let (locked_id, locked_map) = prepare_map(&server, Dimension::OVERWORLD, 239, true);
        player
            .inventory()
            .set_stack_in_hand(Hand::Right, filled_map(updating_id));
        player
            .inventory()
            .set_stack_in_hand(Hand::Left, filled_map(locked_id));
        player.tick_counter.store(1, Ordering::Relaxed);

        player.tick_maps(&server);

        {
            let map_data = updating_map.lock().unwrap();
            assert_ne!(map_data.colors[0], 173);
            assert!(map_data.fully_updated);
        };
        {
            let map_data = locked_map.lock().unwrap();
            assert_eq!(map_data.colors[0], 239);
            assert!(!map_data.dirty);
            assert!(!map_data.fully_updated);
        };

        server.shutdown().await;
    }
}
