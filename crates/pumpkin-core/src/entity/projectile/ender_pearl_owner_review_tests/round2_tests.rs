use super::*;
use crate::plugin::api::events::entity::entity_damage::EntityDamageEvent;
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round2_unloaded_saved_owner_survives_real_pearl_ticks() {
    for cross_world in [false, true] {
        let mut fixture = Fixture::new();
        let player = fixture.owner.player.clone();
        let mut saved = NbtCompound::new();
        saved.put("Health", NbtTag::Float(4.0));
        player.read_nbt_non_mut(&saved);
        player.set_client_loaded(false);
        assert!(!player.has_client_loaded());
        assert_eq!(player.living_entity.health.load(), 4.0);
        let destination = if cross_world {
            fixture.target.clone()
        } else {
            fixture.source.clone()
        };
        fire_test_pearl(&player, &destination, &fixture.server);
        let packets = fixture.finish().await;

        assert!(Arc::ptr_eq(&player.world(), &destination));
        assert_ne!(player.position(), Vector3::new(1.0, 64.0, 1.0));
        assert_eq!(player.living_entity.fall_distance.load(), 0.0);
        assert_eq!(player.living_entity.health.load(), 4.0);
        assert!(!player.living_entity.dead.load(Ordering::Relaxed));
        assert!(
            packets
                .iter()
                .all(|packet| packet_id(packet) != DAMAGE_EVENT.0)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round2_damage_callback_resolves_and_protects_arriving_pearl_owner() {
    use pumpkin_data::packet::clientbound::play::PLAYER_COMBAT_KILL;

    let mut fixture = Fixture::new();
    let player = fixture.owner.player.clone();
    player.living_entity.set_health(4.0);
    let expected_uuid = player.gameprofile.id;
    let server = Arc::downgrade(&fixture.server);
    let resolved = Arc::new(AtomicU32::new(0));
    let observed_resolved = resolved.clone();
    crate::entity::harvest_tests::register::<EntityDamageEvent>(&fixture.server, move |event| {
        if event.damage_type != DamageType::ENDER_PEARL {
            return;
        }
        let server = server.upgrade().unwrap();
        let worlds = server.worlds.load_full();
        let Some(victim) = worlds
            .iter()
            .find_map(|world| world.get_entity_by_id(event.entity_id))
        else {
            return;
        };
        let victim = victim.get_player().unwrap();
        assert_eq!(victim.gameprofile.id, expected_uuid);
        let world = victim.world();
        for found in [
            world.get_player_by_id(event.entity_id),
            world.get_player_by_uuid(expected_uuid),
            server.get_player_by_uuid(expected_uuid),
        ] {
            assert!(std::ptr::eq(victim, found.unwrap().as_ref()));
        }
        assert!(
            world
                .players
                .load()
                .iter()
                .all(|p| p.entity_id() != event.entity_id)
        );
        observed_resolved.fetch_add(1, Ordering::Relaxed);
        event.cancelled = true;
    });
    fixture.fire();
    let packets = fixture.finish().await;

    assert_eq!(resolved.load(Ordering::Relaxed), 1);
    assert_eq!(player.living_entity.health.load(), 4.0);
    assert!(!player.living_entity.dead.load(Ordering::Relaxed));
    assert!(Arc::ptr_eq(&player.world(), &fixture.target));
    assert!(
        fixture
            .target
            .players
            .load()
            .iter()
            .any(|p| Arc::ptr_eq(p, &player))
    );
    assert!(
        packets
            .iter()
            .all(|packet| packet_id(packet) != PLAYER_COMBAT_KILL.0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round2_transfer_loading_permission_does_not_admit_callback_damage() {
    let mut fixture = Fixture::new();
    let player = fixture.owner.player.clone();
    let victim = player.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let observed_calls = calls.clone();
    crate::entity::harvest_tests::register::<EntityDamageEvent>(&fixture.server, move |_| {
        if observed_calls.fetch_add(1, Ordering::Relaxed) != 0 {
            return;
        }
        assert!(!victim.has_client_loaded());
        for damage_type in [DamageType::GENERIC, DamageType::ENDER_PEARL] {
            assert!(!victim.damage(victim.as_ref(), 1.0, damage_type));
        }
    });
    fixture.fire();
    fixture.finish().await;
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(player.living_entity.health.load(), 3.0);
}
