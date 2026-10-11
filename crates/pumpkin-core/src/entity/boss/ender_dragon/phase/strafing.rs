use super::EnderDragonPhase;
use crate::entity::EntityBase;
use crate::entity::{
    Entity,
    boss::ender_dragon::{EnderDragonEntity, Vector3Ext, find_path},
    projectile::dragon_fireball::DragonFireballEntity,
};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

pub struct StrafingPhase;

impl super::Phase for StrafingPhase {
    fn get_type(&self) -> EnderDragonPhase {
        EnderDragonPhase::Strafing
    }

    fn begin(&self, dragon: &EnderDragonEntity) {
        *dragon
            .fireball_charge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0;
        *dragon
            .target_location
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    #[expect(clippy::too_many_lines)]
    fn tick(&self, dragon: &EnderDragonEntity) {
        let target_id = *dragon
            .target_player
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let world = dragon.mob_entity.living_entity.entity.world.load();
        let pos = dragon.mob_entity.living_entity.entity.pos.load();

        let player_target = if let Some(id) = target_id {
            world
                .players
                .load()
                .iter()
                .find(|p| p.gameprofile.id == id)
                .cloned()
        } else {
            None
        };

        let Some(player) = player_target else {
            dragon.set_phase(EnderDragonPhase::Circling);
            return;
        };

        let player_pos = player.get_entity().pos.load();
        let mut path = dragon
            .path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut target_location = dragon
            .target_location
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if path.is_empty() {
            let d2 = player_pos.x - pos.x;
            let d3 = player_pos.z - pos.z;
            let d4 = d2.hypot(d3);
            let d5 = (0.4 + d4 / 80.0 - 1.0).clamp(0.0, 10.0);
            *target_location = Some(Vector3::new(player_pos.x, player_pos.y + d5, player_pos.z));
        }

        let d11 = target_location
            .map(|loc| pos.distance_squared(loc))
            .unwrap_or(0.0);
        if !(100.0..=22500.0).contains(&d11)
            || dragon
                .mob_entity
                .living_entity
                .entity
                .horizontal_collision
                .load(Ordering::Relaxed)
        {
            if path.is_empty() {
                drop(path);
                let i = dragon.find_closest_node();
                let j = dragon.find_closest_node_to(player_pos);

                let mut path_lock = dragon
                    .path
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let nodes = dragon
                    .nodes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *path_lock = find_path(&nodes, i, j, None);
                drop(nodes);
                path = path_lock;
            }

            if let Some(next_node_idx) = path.first().copied() {
                path.remove(0);
                let nodes = dragon
                    .nodes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(node) = nodes[next_node_idx] {
                    let mut y_target = node.y + rand::random_range(0.0..20.0);
                    while y_target < node.y {
                        y_target = node.y + rand::random_range(0.0..20.0);
                    }
                    *target_location = Some(Vector3::new(node.x, y_target, node.z));
                }
            }
        }
        drop(path);
        drop(target_location);

        if player_pos.distance_squared(pos) < 4096.0
            && dragon.get_entity().has_line_of_sight(player.get_entity())
        {
            let mut charge = dragon
                .fireball_charge
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);

            let aim_diff = Vector3::new(player_pos.x - pos.x, 0.0, player_pos.z - pos.z);
            let aim = if aim_diff.length_squared() > 1e-6 {
                aim_diff.normalize()
            } else {
                Vector3::new(0.0, 0.0, 0.0)
            };

            let yaw = dragon.mob_entity.living_entity.entity.yaw.load();
            let dir = Vector3::new(
                (yaw * (std::f32::consts::PI / 180.0)).sin() as f64,
                0.0,
                -(yaw * (std::f32::consts::PI / 180.0)).cos() as f64,
            );

            let dir_norm = if dir.length_squared() > 1e-6 {
                dir.normalize()
            } else {
                Vector3::new(0.0, 0.0, 0.0)
            };

            let dot = dir_norm.dot(&aim) as f32;
            let angle_degs = dot.acos().to_degrees() + 0.5;

            *charge += 1;

            if *charge >= 5 && angle_degs < 10.0 {
                *charge = 0;
                drop(charge);

                Self::launch_fireball(dragon, player.get_entity());

                dragon
                    .path
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
                dragon.set_phase(EnderDragonPhase::Circling);
            }
        } else {
            let mut charge = dragon
                .fireball_charge
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *charge > 0 {
                *charge -= 1;
            }
        }
    }
}

impl StrafingPhase {
    // EnderDragon constructor, line 93: the head's height is 1.0, independently of the dragon type.
    const HEAD_HEIGHT: f64 = 1.0;

    // DragonStrafePlayerPhase.doServerTick launches from the head toward the target's middle.
    fn launch_fireball(dragon: &EnderDragonEntity, target: &Entity) {
        let entity = dragon.get_entity();
        let world = entity.world.load();
        let view = entity.rotation().to_f64();
        let head = &dragon.parts[0].entity;
        let mut start = head.pos.load();
        start.x -= view.x;
        start.y += Self::HEAD_HEIGHT * 0.5 + 0.5;
        start.z -= view.z;
        let mut aim = target.pos.load();
        aim.y += f64::from(target.entity_dimension.load().height) * 0.5;
        let fireball = DragonFireballEntity::new_shot(
            Entity::new(world.clone(), start, &EntityType::DRAGON_FIREBALL),
            entity,
            aim - start,
        );
        if !entity.silent.load(Ordering::Relaxed) {
            world.broadcast_to_chunk(
                entity.chunk_pos.load(),
                &pumpkin_protocol::java::client::play::CWorldEvent::new(
                    1017,
                    entity.block_pos.load(),
                    0,
                    false,
                ),
            );
        }
        world.spawn_entity(std::sync::Arc::new(fireball));
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use pumpkin_data::Block;
    use pumpkin_util::math::vector2::Vector2;

    #[tokio::test]
    async fn dragon_strafe_launches_a_fireball_and_cloud_waits_for_impact() {
        let dir = tempfile::tempdir().unwrap();
        let server = crate::server::combat_test_support::server(dir.path());
        let world = crate::server::combat_test_support::world(&server, dir.path());
        world.level.loaded_chunks.insert(
            Vector2::new(0, 0),
            pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
        );
        let player = crate::net::java::combat_test_support::TestPlayer::new(&world).player;
        player.get_entity().set_pos(Vector3::new(8.5, 64.0, 2.5));
        let dragon = EnderDragonEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.5, 64.0, 12.5),
            &EntityType::ENDER_DRAGON,
        ));
        dragon.parts[0]
            .entity
            .set_pos(Vector3::new(8.5, 63.5, 10.5));
        *dragon.target_player.lock().unwrap() = Some(player.get_entity().entity_uuid);
        *dragon.fireball_charge.lock().unwrap() = 4;
        world
            .entities
            .store(std::sync::Arc::new(vec![dragon.clone()]));
        super::super::Phase::tick(&StrafingPhase, &dragon);
        let entities = world.entities.load_full();
        assert!(
            entities
                .iter()
                .all(|entity| entity.get_entity().entity_type != &EntityType::AREA_EFFECT_CLOUD)
        );
        let fireball = entities
            .iter()
            .find(|entity| entity.get_entity().entity_type == &EntityType::DRAGON_FIREBALL)
            .unwrap();
        assert!(fireball.get_entity().pos.load().z > 8.0);
        assert_eq!(fireball.get_entity().pos.load().y, 64.5);
        // Build cover after launch, then use real projectile ticks to reach the wall.
        let chunk = world.level.loaded_chunks.get(&Vector2::new(0, 0)).unwrap();
        chunk.set_block_absolute_y(8, 64, 6, Block::STONE.default_state.id);
        drop(chunk);
        for _ in 0..40 {
            fireball.tick(fireball.as_ref(), &server);
            if fireball.get_entity().is_removed() {
                break;
            }
        }
        let hit = fireball.get_entity().pos.load();
        assert!((hit.z - 7.0).abs() < 1e-6);
        let clouds = world.entities.load();
        let cloud = clouds
            .iter()
            .find(|entity| entity.get_entity().entity_type == &EntityType::AREA_EFFECT_CLOUD)
            .unwrap();
        assert_eq!(cloud.get_entity().pos.load(), hit);
        assert!(fireball.get_entity().is_removed());
        crate::server::fixture_lifecycle::finish().await;
    }
}
