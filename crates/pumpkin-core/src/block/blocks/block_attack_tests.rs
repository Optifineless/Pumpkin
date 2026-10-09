use super::*;
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::block::{block_damage::BlockDamageEvent, note_play::NotePlayEvent},
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block,
    biome::Biome,
    statistic::{CustomStatistic, StatisticCategory},
};
use pumpkin_protocol::{
    VarInt,
    java::{client::play::CWorldEvent, server::play::SPlayerAction},
};
use pumpkin_util::{GameMode, math::vector3::Vector3, permission::PermissionLvl};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn attack(player: &TestPlayer, position: BlockPos, server: &Arc<Server>) {
    player.client().handle_player_action(
        &player.player,
        &SPlayerAction {
            status: VarInt(0),
            position,
            face: 1,
            sequence: VarInt(1),
        },
        server,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_egg_attack_moves_without_drop_and_within_bounds() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let mut player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    let position = BlockPos::new(8, 64, 8);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    {
        let mut border = world.worldborder.lock().unwrap();
        border.center_x = 8.0;
        border.center_z = 8.0;
        border.new_diameter = 8.0;
    }
    world.set_block_state(
        &position,
        Block::DRAGON_EGG.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    player.take_packets();
    attack(&player, position, &fixture.server);
    assert!(world.get_block_state(&position).is_air());
    let eggs: Vec<_> = (0..16)
        .flat_map(|x| (0..16).map(move |z| BlockPos::new(x, 64, z)))
        .filter(|pos| world.get_block(pos) == &Block::DRAGON_EGG)
        .collect();
    assert_eq!(eggs.len(), 1);
    let target = eggs[0];
    assert!(
        world
            .worldborder
            .lock()
            .unwrap()
            .contains(f64::from(target.0.x), f64::from(target.0.z))
    );
    let delta = target.0 - position.0;
    assert!(delta.x.abs() <= 15 && delta.y.abs() <= 7 && delta.z.abs() <= 15);
    let packed =
        ((delta.x + 16) & 0xff) << 16 | ((delta.y + 8) & 0xff) << 8 | ((delta.z + 16) & 0xff);
    let effect = player
        .client()
        .serialize_packet(&CWorldEvent::new(2015, position, packed, false))
        .unwrap();
    assert_eq!(
        player
            .take_packets()
            .iter()
            .filter(|packet| **packet == effect)
            .count(),
        1
    );
    assert!(
        world
            .entities
            .load()
            .iter()
            .all(|entity| entity.get_item_entity().is_none())
    );
    fixture.server.shutdown().await;
}

struct AttackControl {
    cancel: AtomicBool,
    notes: AtomicUsize,
}
impl EventHandler<BlockDamageEvent> for AttackControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut BlockDamageEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = self.cancel.load(Ordering::Relaxed);
        Box::pin(async {})
    }
}
impl EventHandler<NotePlayEvent> for AttackControl {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut NotePlayEvent,
    ) -> BoxFuture<'a, ()> {
        self.notes.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn note_block_attack_only_once_after_restrictions() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestPlayer::new(&world);
    player.player.permission_lvl.store(PermissionLvl::Four);
    let position = BlockPos::new(8, 64, 8);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.5, 64.0, 8.5));
    let control = Arc::new(AttackControl {
        cancel: AtomicBool::new(true),
        notes: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<BlockDamageEvent, _>(control.clone(), EventPriority::Normal, true);
    fixture.server.plugin_manager.register::<NotePlayEvent, _>(
        control.clone(),
        EventPriority::Normal,
        true,
    );
    world.set_block_state(
        &position,
        Block::NOTE_BLOCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    attack(&player, position, &fixture.server);
    control.cancel.store(false, Ordering::Relaxed);
    for mode in [GameMode::Spectator, GameMode::Adventure] {
        player.player.set_gamemode(mode);
        attack(&player, position, &fixture.server);
    }
    assert_eq!(control.notes.load(Ordering::Relaxed), 0);
    player.player.set_gamemode(GameMode::Survival);
    attack(&player, position, &fixture.server);
    assert_eq!(control.notes.load(Ordering::Relaxed), 1);
    assert_eq!(
        player.player.stats.lock().unwrap().get(
            StatisticCategory::Custom,
            CustomStatistic::PlayNoteblock as i32
        ),
        1
    );
    player.player.set_gamemode(GameMode::Creative);
    attack(&player, position, &fixture.server);
    assert_eq!(control.notes.load(Ordering::Relaxed), 1);
    assert!(world.get_block_state(&position).is_air());
    fixture.server.shutdown().await;
}
