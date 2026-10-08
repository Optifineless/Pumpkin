use super::rabbit::{RabbitEntity, RabbitVariant};
use crate::{
    entity::{
        Entity, EntityBase,
        ai::pathfinder::{node::Node, path::Path},
        mob::Mob,
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
};
use pumpkin_data::{
    Block,
    entity::EntityType,
    sound::{Sound, SoundCategory},
};
use pumpkin_protocol::ser::NetworkReadExt;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::chunk::ChunkData;
use std::sync::atomic::Ordering::Relaxed;

fn rabbit(world: &std::sync::Arc<crate::world::World>) -> std::sync::Arc<RabbitEntity> {
    let rabbit = RabbitEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 64.0, 8.5),
        &EntityType::RABBIT,
    ));
    rabbit.get_entity().on_ground.store(true, Relaxed);
    rabbit
}

fn sounds(witness: &mut TestPlayer, sound: Sound) -> Vec<(i32, f32, f32)> {
    witness
        .take_packets()
        .into_iter()
        .filter_map(|bytes| {
            let mut data = bytes.as_ref();
            if data.get_var_int().unwrap().0 != pumpkin_data::packet::clientbound::play::SOUND.0
                || data.get_var_int().unwrap().0 != sound as i32 + 1
            {
                return None;
            }
            let category = data.get_var_int().unwrap().0;
            for _ in 0..3 {
                data.get_i32_be().unwrap();
            }
            Some((category, data.get_f32().unwrap(), data.get_f32().unwrap()))
        })
        .collect()
}

#[tokio::test]
async fn rabbit_review_path_node_jump_height() {
    let directory = tempfile::tempdir().unwrap();
    let world = crate::entity::living::test_support::armor_test_world(directory.path());
    let chunk = ChunkData::empty_sync(0, 0);
    chunk.set_block_absolute_y(10, 64, 8, Block::STONE_SLAB.default_state.id);
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let rabbit = rabbit(&world);
    let node = BlockPos::new(10, 65, 8);
    let mut navigation = rabbit.mob_entity.navigator.lock().unwrap();
    assert!(navigation.move_to_path(
        Some(Path::new(vec![Node::new(node)], node, true)),
        0.6,
        &rabbit.mob_entity.living_entity,
    ));
    navigation.tick(&rabbit.mob_entity, rabbit.as_ref());
    assert_eq!(navigation.next_move_target().unwrap().0.y, 64.5);
    drop(navigation);
    rabbit
        .mob_entity
        .move_control
        .lock()
        .unwrap()
        .set_wanted_position(10.5, 64.0, 8.5, 0.6);
    assert!(!rabbit.mob_entity.living_entity.jumping.load(Relaxed));
    assert!((rabbit.rabbit_jump_power_scale() - 1.190_476_2).abs() < 1e-7);

    // A finished path cannot request height; wanted Y remains an independent condition.
    rabbit
        .mob_entity
        .navigator
        .lock()
        .unwrap()
        .get_path_mut()
        .unwrap()
        .advance();
    assert!((rabbit.rabbit_jump_power_scale() - 0.476_190_5).abs() < 1e-7);
    rabbit
        .mob_entity
        .move_control
        .lock()
        .unwrap()
        .set_wanted_position(10.5, 65.0, 8.5, 0.6);
    rabbit.mob_entity.living_entity.jumping.store(true, Relaxed);
    assert!((rabbit.rabbit_jump_power_scale() - 1.190_476_2).abs() < 1e-7);
}

#[tokio::test]
async fn rabbit_review_killer_target_order() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let mut witness = TestPlayer::new(&world);
    witness
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    for had_wanted in [true, false] {
        let rabbit = rabbit(&world);
        rabbit.set_variant(RabbitVariant::Evil);
        rabbit.tick_hopping();
        rabbit.jump_state.delay.store(0, Relaxed);
        {
            let mut control = rabbit.mob_entity.move_control.lock().unwrap();
            control.set_wanted_position(8.5, 64.0, 12.5, 1.4);
            if !had_wanted {
                control.tick(rabbit.as_ref());
            }
            assert_eq!(control.has_wanted(), had_wanted);
        };
        assert!(
            rabbit
                .mob_entity
                .navigator
                .lock()
                .unwrap()
                .get_path()
                .is_none()
        );
        rabbit.mob_entity.set_target(Some(witness.player.clone()));
        witness.take_packets();
        rabbit.tick_hopping();
        assert_eq!(rabbit.get_entity().yaw.load(), -90.0);
        assert_eq!(
            rabbit
                .mob_entity
                .move_control
                .lock()
                .unwrap()
                .wanted_position()
                .0,
            witness.player.get_entity().pos.load()
        );
        assert_eq!(sounds(&mut witness, Sound::EntityRabbitJump).len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rabbit_review_successful_hit_sound() {
    let directory = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(directory.path());
    let world = combat_test_support::world(&server, directory.path());
    let mut witness = TestPlayer::new(&world);
    let rabbit = rabbit(&world);
    rabbit.set_variant(RabbitVariant::Evil);
    witness.take_packets();
    let health = witness.player.living_entity.health.load();
    rabbit
        .mob_entity
        .try_attack(rabbit.as_ref(), witness.player.as_ref());
    assert!(witness.player.living_entity.health.load() < health);
    let attacks = sounds(&mut witness, Sound::EntityRabbitAttack);
    assert_eq!(attacks.len(), 1);
    assert_eq!(attacks[0].0, SoundCategory::Hostile as i32);
    assert_eq!(attacks[0].1, 1.0);
    assert!((0.8..=1.2).contains(&attacks[0].2));
    // Hurt cooldown rejects the second hit, so the attack sound must not repeat.
    rabbit
        .mob_entity
        .try_attack(rabbit.as_ref(), witness.player.as_ref());
    assert!(sounds(&mut witness, Sound::EntityRabbitAttack).is_empty());
    rabbit.get_entity().set_silent(true);
    rabbit.play_attack_sound();
    assert!(sounds(&mut witness, Sound::EntityRabbitAttack).is_empty());
    rabbit.get_entity().set_silent(false);
    rabbit.set_variant(RabbitVariant::Brown);
    rabbit.play_attack_sound();
    assert!(sounds(&mut witness, Sound::EntityRabbitAttack).is_empty());
}
