#![expect(
    clippy::unwrap_used,
    reason = "Explosion regression fixtures must be valid"
)]
use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{BoxFuture, EventHandler, EventPriority, entity::entity_damage::EntityDamageEvent},
    server::{
        Server,
        combat_test_support::{server, world},
    },
};
struct Reset(Arc<crate::entity::player::Player>);
impl EventHandler<EntityDamageEvent> for Reset {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityDamageEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.0.entity_id() {
            self.0.living_entity.reset_state();
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_explosion_damage_reset_discards_followup_impulse() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    victim.get_entity().set_pos(Vector3::new(8.5, 100.0, 8.5));
    world.level.loaded_chunks.insert(
        pumpkin_util::math::vector2::Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );
    server.plugin_manager.register::<EntityDamageEvent, _>(
        Arc::new(Reset(victim.clone())),
        EventPriority::Normal,
        true,
    );
    let explosion = Explosion::new(
        2.0,
        Vector3::new(8.5, 100.0, 9.5),
        super::super::BlockInteraction::Keep,
    );
    let result = explosion.damage_entities(&world);
    assert!(!result.player_knockback.contains_key(&victim.entity_id()));
    assert_eq!(victim.get_entity().velocity.load(), Vector3::default());
    assert_eq!(victim.living_entity.health.load(), 20.0);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification3_explosion_reset_before_packet_discards_queued_impulse() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    victim.get_entity().set_pos(Vector3::new(8.5, 100.0, 8.5));
    world.level.loaded_chunks.insert(
        pumpkin_util::math::vector2::Vector2::new(0, 0),
        pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
    );
    let explosion = Explosion::new(
        2.0,
        Vector3::new(8.5, 100.0, 9.5),
        super::super::BlockInteraction::Keep,
    );
    let result = explosion.damage_entities(&world);
    victim.living_entity.with_damage_owned(|| {
        assert!(
            result
                .player_knockback_for(&victim)
                .unwrap()
                .length_squared()
                > 0.0
        );
        victim.living_entity.reset_state();
        assert!(result.player_knockback_for(&victim).is_none());
    });
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification4_far_explosion_does_not_acquire_player() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let victim = TestPlayer::new(&world).player;
    world.players.store(Arc::new(vec![victim.clone()]));
    // Inside the search box but outside the spherical damage radius.
    victim.get_entity().set_pos(Vector3::new(12.0, 104.0, 12.0));
    let before = victim.living_entity.damage_entry_count();
    let explosion = Explosion::new(
        2.0,
        Vector3::new(8.0, 100.0, 8.0),
        super::super::BlockInteraction::Keep,
    );
    assert!(
        explosion
            .damage_entities(&world)
            .player_lifecycles
            .is_empty()
    );
    let damage_entries = victim.living_entity.damage_entry_count() - before;
    victim
        .get_entity()
        .set_pos(Vector3::new(100.0, 100.0, 100.0));
    let before = victim.living_entity.damage_entry_count();
    world.run_explosion(&explosion);
    let packet_entries = victim.living_entity.damage_entry_count() - before;
    assert_eq!((damage_entries, packet_entries), (0, 0));
    crate::server::fixture_lifecycle::finish().await;
}
