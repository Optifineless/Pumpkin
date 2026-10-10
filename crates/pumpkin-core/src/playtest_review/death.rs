use super::*;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::damage::DamageType;

struct RespawnPickup {
    drop: Arc<dyn EntityBase>,
    calls: std::sync::atomic::AtomicUsize,
}

impl
    crate::plugin::EventHandler<
        crate::plugin::api::events::player::player_respawn::PlayerRespawnEvent,
    > for RespawnPickup
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::player_respawn::PlayerRespawnEvent,
    ) -> crate::plugin::BoxFuture<'a, ()> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Even after the death delay expires, the old life cannot collect during respawn.
        assert_eq!(self.drop.get_item_entity().unwrap().get_pickup_delay(), 0);
        self.drop.on_player_collision(&event.player);
        assert!(self.drop.get_entity().is_alive());
        assert!(
            event
                .player
                .inventory
                .main_inventory
                .read()
                .unwrap()
                .iter()
                .all(ItemStack::is_empty)
        );
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn death_drops_remain_after_waiting_beyond_forty_ticks_and_respawning() {
    let fixture = Fixture::new();
    let mut victim = TestPlayer::new(&fixture.world);
    victim
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 64.0, 8.0));
    victim
        .player
        .inventory
        .set_stack(0, ItemStack::new(3, &Item::DIAMOND));
    fixture.world.set_block_state(
        &BlockPos::new(8, 63, 8),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    victim
        .player
        .damage(victim.player.as_ref(), 100.0, DamageType::GENERIC_KILL);
    let drop = fixture
        .world
        .entities
        .load()
        .iter()
        .find(|e| e.get_item_entity().is_some())
        .unwrap()
        .clone();
    let item = drop.get_item_entity().unwrap();
    assert_eq!(item.get_pickup_delay(), 40);
    for _ in 0..41 {
        item.tick(item, &fixture.server);
    }
    assert_eq!(item.get_pickup_delay(), 0);
    item.on_player_collision(&victim.player);
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    fixture.world.set_block_state(
        &BlockPos::new(12, 63, 12),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    victim.player.set_respawn_point(
        pumpkin_data::dimension::Dimension::OVERWORLD,
        BlockPos::new(12, 64, 12),
        0.0,
        0.0,
        true,
    );
    let callback = Arc::new(RespawnPickup {
        drop: drop.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    fixture.server.plugin_manager.register(
        callback.clone(),
        crate::plugin::EventPriority::Normal,
        true,
    );
    let player = victim.player.clone();
    let respawn = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.world.respawn_player(&player, false),
    );
    tokio::pin!(respawn);
    loop {
        tokio::select! {
            result = &mut respawn => { result.unwrap(); break; }
            packet = victim.packets.recv() => {
                use crate::net::java::outgoing::{Completion, OutgoingPacket};
                if let Some(OutgoingPacket::Data { completion: Some(Completion::Framed(done) | Completion::Flushed(done)), .. }) = packet {
                    let _ = done.send(());
                }
            }
        }
    }
    assert_eq!(callback.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(victim.player.inventory.get_stack(0).item_count, 0);
    assert_eq!(fixture.drops(&Item::DIAMOND), 3);
    assert!(victim.player.living_entity.health.load() > 0.0);
    fixture.shutdown().await;
}
