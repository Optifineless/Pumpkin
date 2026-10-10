#![expect(
    clippy::unwrap_used,
    reason = "Motion regression fixtures must be valid"
)]

use super::*;
use crate::{
    entity::Entity,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{
    Enchantment, item::Item, item_stack::ItemStack, packet::clientbound::play::SET_ENTITY_MOTION,
};
use pumpkin_protocol::{codec::lp_vector_3d::LpVector3d, ser::NetworkReadExt};

pub(super) fn motions(fixture: &mut TestPlayer, entity_id: i32) -> Vec<Vector3<f64>> {
    fixture
        .take_packets()
        .iter()
        .filter_map(|bytes| {
            let mut data = bytes.as_ref();
            if data.get_var_int().unwrap().0 != SET_ENTITY_MOTION.0 {
                return None;
            }
            let id = data.get_var_int().unwrap().0;
            (id == entity_id).then(|| {
                // LpVec3.read consumes only the single sentinel byte for zero motion.
                if data == [0] {
                    Vector3::default()
                } else {
                    LpVector3d::read(&mut data).unwrap().0
                }
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_skeleton_arrow_motion_is_one_capped_packet_and_never_replayed_by_pushes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let skeleton = crate::entity::r#type::from_type(
        &EntityType::SKELETON,
        Vector3::new(0.5, 64.0, 1.0),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(skeleton.clone());
    for (grounded, punch, initial_y, expected_y) in [
        (true, 0, 0.0, 0.4),
        (true, 2, 0.0, 0.5),
        (false, 0, -0.2, -0.2),
    ] {
        player.living_entity.reset_state();
        player
            .living_entity
            .hurt_cooldown
            .store(0, Ordering::Relaxed);
        player.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));
        player
            .get_entity()
            .on_ground
            .store(grounded, Ordering::Relaxed);
        player
            .get_entity()
            .velocity
            .store(Vector3::new(0.0, initial_y, 0.0));
        let arrow = ArrowEntity::new(
            Entity::new(
                world.clone(),
                Vector3::new(0.5, 64.5, 1.0),
                &EntityType::ARROW,
            ),
            Some(skeleton.get_entity().entity_id),
        );
        let mut bow = ItemStack::new(1, &Item::BOW);
        if punch > 0 {
            bow.add_enchantment(&Enchantment::PUNCH, punch);
        }
        *arrow.weapon.write().unwrap() = Some(bow);
        arrow.entity.velocity.store(Vector3::new(0.0, 0.0, -1.6));
        fixture.take_packets();
        arrow.hit_entity(&(player.clone() as Arc<dyn EntityBase>), player.position());
        assert!(
            motions(&mut fixture, player.entity_id()).is_empty(),
            "arrow follow-ups must finish before the tick delivers motion"
        );
        player.living_entity.flush_player_motion();
        let packets = motions(&mut fixture, player.entity_id());
        assert_eq!(
            packets.len(),
            1,
            "base knockback and Punch must share one delivery"
        );
        assert!((packets[0].y - expected_y).abs() < 0.000_04);
        if grounded && punch == 0 {
            assert!(packets[0].y <= 0.4);
        }
        for _ in 0..20 {
            skeleton.push(player.as_ref());
            player.living_entity.flush_player_motion();
            assert!(
                motions(&mut fixture, player.entity_id()).is_empty(),
                "horizontal collision replayed vertical hurt motion"
            );
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtest_summoned_zombie_push_does_not_resend_upward_melee_motion() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let player = fixture.player.clone();
    let mut observer = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![player.clone(), observer.player.clone()]));
    world
        .entity_tracker
        .get_tracked_entity(player.entity_id())
        .unwrap()
        .seen_by
        .insert(observer.player.gameprofile.id);
    player.get_entity().set_pos(Vector3::new(0.5, 64.0, 0.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    player.get_entity().velocity.store(Vector3::default());
    let zombie = crate::entity::r#type::from_type(
        &EntityType::ZOMBIE,
        player.position(),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(zombie.clone());
    fixture.take_packets();
    observer.take_packets();
    zombie
        .get_mob()
        .unwrap()
        .get_mob_entity()
        .try_attack(zombie.as_ref(), player.as_ref());
    player.living_entity.flush_player_motion();
    let packets = motions(&mut fixture, player.entity_id());
    assert_eq!(packets.len(), 1);
    assert!(packets[0].y <= 0.4);
    assert_eq!(motions(&mut observer, player.entity_id()).len(), 1);
    player
        .get_entity()
        .on_ground
        .store(false, Ordering::Relaxed);
    zombie
        .get_entity()
        .set_pos(player.position() + Vector3::new(0.1, 0.0, 0.1));
    for _ in 0..20 {
        zombie.push(player.as_ref());
        player.living_entity.flush_player_motion();
        assert!(
            motions(&mut fixture, player.entity_id()).is_empty(),
            "summoned mob collision resent an old upward impulse"
        );
        assert_eq!(motions(&mut observer, player.entity_id()).len(), 1);
    }
    crate::server::fixture_lifecycle::finish().await;
}
