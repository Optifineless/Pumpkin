use super::{tests::hook, *};
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::projectile_hit::ProjectileHitEvent,
    },
    server::combat_test_support::{server, world},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, Enchantment, biome::Biome, entity::EntityType, item_stack::ItemStack};
use pumpkin_protocol::{VarInt, java::server::play::SUseItem, ser::NetworkReadExt};
use std::sync::{Arc, atomic::AtomicUsize};

#[derive(Default)]
struct HitCounter(AtomicUsize);

impl EventHandler<ProjectileHitEvent> for HitCounter {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ProjectileHitEvent,
    ) -> BoxFuture<'a, ()> {
        assert!(event.hit_entity_id.is_none());
        self.0.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_hook_resting_on_target_fires_one_hit_in_40_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 8.5));
    let mut chunk = proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(8, 63, 8, Block::TARGET.default_state);
    publish(&world, chunk);
    let bobber = hook(&world, &owner.player);
    bobber.entity.set_pos(Vector3::new(8.5, 65.0, 8.5));
    bobber.entity.velocity.store(Vector3::new(0.0, -2.0, 0.0));
    let hits = Arc::new(HitCounter::default());
    server.plugin_manager.register::<ProjectileHitEvent, _>(
        hits.clone(),
        EventPriority::Normal,
        true,
    );
    for _ in 0..40 {
        bobber.process_tick(&bobber);
    }
    assert_eq!(bobber.entity.pos.load(), Vector3::new(8.5, 64.0, 8.5));
    assert_eq!(hits.0.load(Relaxed), 1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_hook_cast_onto_stone_survives_1300_ticks_with_a_rod() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 8.5));
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let bobber = Arc::new(hook(&world, &owner.player));
    world.spawn_entity(bobber.clone());
    owner
        .player
        .fishing_bobber
        .store(bobber.entity.entity_id, Relaxed);
    bobber.entity.set_pos(Vector3::new(8.5, 65.0, 8.5));
    bobber.entity.velocity.store(Vector3::new(0.0, -2.0, 0.0));
    for _ in 0..1300 {
        bobber.process_tick(bobber.as_ref());
        assert!(!bobber.entity.is_removed());
    }
    assert_eq!(bobber.entity.pos.load(), Vector3::new(8.5, 64.0, 8.5));
    assert_eq!(
        owner.player.fishing_bobber.load(Relaxed),
        bobber.entity.entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_entity_collision_requires_positive_ray_distance() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let target = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![owner.player.clone(), target.player.clone()]));
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    target
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 3.0));
    let bobber = hook(&world, &owner.player);
    bobber.projectile.left_owner.store(true, Relaxed);
    let front = target.player.get_entity().bounding_box.load().min.z;
    bobber.entity.set_pos(Vector3::new(0.0, 65.0, front));
    let mut velocity = Vector3::new(0.0, 0.0, 1.0);
    bobber.check_collision(&world, &mut velocity);
    assert_eq!(bobber.hooked_entity_id.load(Relaxed), -1);
    bobber.entity.set_pos(Vector3::new(0.0, 65.0, front - 0.1));
    bobber.check_collision(&world, &mut velocity);
    assert_eq!(
        bobber.hooked_entity_id.load(Relaxed),
        target.player.get_entity().entity_id
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fishing_short_block_rays_do_not_fire_hits() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    publish(&world, proto(&Biome::PLAINS, &Block::TARGET));
    let bobber = hook(&world, &owner.player);
    bobber.entity.set_pos(Vector3::new(8.5, 64.0, 8.5));
    let hits = Arc::new(HitCounter::default());
    server.plugin_manager.register::<ProjectileHitEvent, _>(
        hits.clone(),
        EventPriority::Normal,
        true,
    );
    for y in [0.0, -0.0001] {
        bobber.check_collision(&world, &mut Vector3::new(0.0, y, 0.0));
    }
    assert_eq!(hits.0.load(Relaxed), 0);
    crate::server::fixture_lifecycle::finish().await;
}

pub(super) fn tracked_value(entity: &Entity, tracked: tracked_data::TrackedData) -> Vec<u8> {
    let version = pumpkin_data::packet::CURRENT_MC_VERSION;
    let bytes = entity
        .synched_data
        .pack_dirty_for_version(&version)
        .unwrap();
    let mut reader = bytes.as_ref();
    assert_eq!(reader.get_u8().unwrap(), tracked.id.get(&version));
    assert_eq!(reader.get_var_int().unwrap().0, tracked.r#type.id(version));
    assert_eq!(reader.last(), Some(&255));
    reader[..reader.len() - 1].to_vec()
}

fn cast_enchanted_hook(
    server: &Arc<Server>,
    world: &Arc<World>,
    owner: &TestPlayer,
) -> Arc<dyn EntityBase> {
    let mut rod = ItemStack::new(1, &Item::FISHING_ROD);
    rod.add_enchantment(&Enchantment::LUCK_OF_THE_SEA, 3);
    rod.add_enchantment(&Enchantment::LURE, 1);
    owner.player.inventory().set_stack_in_hand(Hand::Right, rod);
    owner.client().handle_use_item(
        &owner.player,
        &SUseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        server,
    );
    world
        .get_entity_by_id(owner.player.fishing_bobber.load(Relaxed))
        .unwrap()
}

#[tokio::test]
async fn fishing_full_tick_sequence_reads_rod_enchantments_bites_and_retrieves_real_loot() {
    // FishingRodItem.use, then FishingHook.tick/catchingFish/retrieve through the real table.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 8.5));
    let hook = cast_enchanted_hook(&server, &world, &owner);
    let rod = owner.player.inventory().held_item();
    let bobber = hook
        .cast_any()
        .downcast_ref::<FishingBobberEntity>()
        .unwrap();
    assert_eq!(bobber.luck, 3);
    assert_eq!(bobber.lure_speed, 100);
    let mut chunk = proto(&Biome::PLAINS, &Block::AIR);
    for x in 6..=10 {
        for z in 6..=10 {
            for y in 63..=64 {
                chunk.set_block_state(x, y, z, Block::WATER.default_state);
            }
        }
    }
    publish(&world, chunk);
    bobber.entity.set_pos(Vector3::new(8.5, 65.5, 8.5));
    bobber.entity.velocity.store(Vector3::new(0.0, -1.0, 0.0));
    bobber.process_tick(bobber);
    assert_eq!(bobber.state.load(), HookState::Flying);
    assert!(bobber.entity.pos.load().y < 65.0);
    let incoming = bobber.entity.velocity.load();
    bobber.process_tick(bobber);
    assert_eq!(bobber.state.load(), HookState::Bobbing);
    assert_eq!(
        bobber.entity.velocity.load(),
        incoming.multiply(0.3, 0.2, 0.3)
    );
    bobber.entity.set_pos(Vector3::new(8.5, 64.5, 8.5));
    bobber.entity.velocity.store(Vector3::default());
    bobber.process_tick(bobber);
    assert!((0..=500).contains(&bobber.wait_countdown.load(Relaxed)));
    bobber.wait_countdown.store(1, Relaxed);
    bobber.process_tick(bobber);
    assert!((20..=80).contains(&bobber.hook_countdown.load(Relaxed)));
    bobber.hook_countdown.store(1, Relaxed);
    bobber.entity.synched_data.clear_dirty();
    bobber.process_tick(bobber);
    assert!((20..=40).contains(&bobber.bite_countdown.load(Relaxed)));
    assert_eq!(
        tracked_value(&bobber.entity, tracked_data::fishing_bobber::DATA_BITING),
        [1]
    );
    assert!(bobber.entity.velocity.load().y < 0.0);
    assert_eq!(bobber.reel_in(&owner.player, &rod, Hand::Right), 1);
    assert!(bobber.entity.is_removed());
    let entities = world.entities.load();
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity.get_item_entity().is_some())
            .count(),
        1
    );
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity.get_entity().entity_type == &EntityType::EXPERIENCE_ORB)
            .count(),
        1
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_hook_tracks_entity_id_and_follows_at_eighty_percent_height() {
    // FishingHook.setHookedEntity and tick's hookedIn.getY(0.8).
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let target = TestPlayer::new(&world);
    world
        .players
        .store(Arc::new(vec![owner.player.clone(), target.player.clone()]));
    let bobber = hook(&world, &owner.player);
    bobber.entity.synched_data.clear_dirty();
    let id = target.player.get_entity().entity_id;
    bobber.set_hooked_entity(Some(id));
    let bytes = tracked_value(&bobber.entity, tracked_data::fishing_bobber::HOOKED_ENTITY);
    assert_eq!(bytes.as_slice().get_var_int().unwrap().0, id + 1);
    bobber.process_tick(&bobber);
    target
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 70.0, 7.0));
    bobber.process_tick(&bobber);
    let pos = bobber.entity.pos.load();
    assert_eq!(pos.x, 4.0);
    assert!((pos.y - 71.44).abs() < 1e-6);
    assert_eq!(pos.z, 7.0);
    bobber.entity.synched_data.clear_dirty();
    bobber.set_hooked_entity(None);
    assert_eq!(
        tracked_value(&bobber.entity, tracked_data::fishing_bobber::HOOKED_ENTITY),
        [0]
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_owner_dimension_change_discards_hook_and_clears_reference() {
    use pumpkin_data::dimension::Dimension;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let bobber = hook(&world, &owner.player);
    owner
        .player
        .fishing_bobber
        .store(bobber.entity.entity_id, Relaxed);
    let destination = Arc::new(World::load(
        pumpkin_world::level::Level::from_root_folder(
            &pumpkin_config::world::LevelConfig::default(),
            dir.path().join("nether"),
            0,
            Dimension::THE_NETHER,
        ),
        server.level_info.clone(),
        Dimension::THE_NETHER,
        server.block_registry.clone(),
        Arc::downgrade(&server),
    ));
    crate::server::fixture_lifecycle::track_world(&destination);
    owner.player.get_entity().world.store(destination);
    bobber.process_tick(&bobber);
    assert!(bobber.entity.is_removed());
    assert_eq!(owner.player.fishing_bobber.load(Relaxed), -1);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_projectile_owner_persists_resolves_from_cache_and_clears_the_current_player() {
    // Projectile save/load and FishingHook.getPlayerOwner/updateOwnerInfo share one owner.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let replacement = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        owner.player.clone(),
        replacement.player.clone(),
    ]));
    let bobber = hook(&world, &owner.player);
    let id = bobber.entity.entity_id;
    owner.player.fishing_bobber.store(id, Relaxed);
    assert_eq!(
        bobber.get_owner_id(),
        Some(owner.player.get_entity().entity_id)
    );
    let lookups = bobber.projectile.owner_lookups.load(Relaxed);
    assert!(Arc::ptr_eq(
        &bobber.get_player_owner().unwrap(),
        &owner.player
    ));
    assert_eq!(bobber.projectile.owner_lookups.load(Relaxed), lookups);

    bobber.projectile.left_owner.store(true, Relaxed);
    bobber.projectile.tick(&bobber.entity);
    let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
    EntityBase::write_nbt(&bobber, &mut nbt);
    assert_eq!(
        nbt.get_uuid("Owner"),
        Some(owner.player.get_entity().entity_uuid)
    );
    assert_eq!(nbt.get_bool("LeftOwner"), Some(true));
    assert_eq!(nbt.get_bool("HasBeenShot"), Some(true));
    nbt.put_uuid("Owner", replacement.player.get_entity().entity_uuid);
    EntityBase::read_nbt_non_mut(&bobber, &nbt);
    assert!(Arc::ptr_eq(
        &bobber.get_player_owner().unwrap(),
        &replacement.player
    ));
    replacement.player.fishing_bobber.store(id, Relaxed);
    bobber.bite_countdown.store(20, Relaxed);
    // The saved owner has no rod, even though the original caster can still reel in.
    assert_eq!(
        bobber.reel_in(
            &owner.player,
            &owner.player.inventory().held_item(),
            Hand::Right
        ),
        0
    );
    assert!(bobber.entity.is_removed());
    assert_eq!(replacement.player.fishing_bobber.load(Relaxed), -1);
    assert_eq!(owner.player.fishing_bobber.load(Relaxed), id);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_non_player_projectile_owner_discards_the_hook() {
    // FishingHook.tick requires getPlayerOwner, even when Projectile.getOwner resolves an entity.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    let bobber = hook(&world, &owner.player);
    let mob = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        owner.player.position(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![mob.clone()]));
    bobber.projectile.set_owner(Some(mob.get_entity()));
    assert!(bobber.projectile_owner().is_some());
    assert!(bobber.get_player_owner().is_none());
    assert_eq!(
        bobber.reel_in(
            &owner.player,
            &owner.player.inventory().held_item(),
            Hand::Right
        ),
        0
    );
    assert!(!bobber.entity.is_removed());
    bobber.process_tick(&bobber);
    assert!(bobber.entity.is_removed());
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn fishing_hook_deflects_from_breeze_before_hooking_it() {
    // FishingHook.checkCollision -> Projectile.hitTargetOrDeflectSelf must keep deflected motion.
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world);
    owner
        .player
        .get_entity()
        .set_pos(Vector3::new(0.0, 64.0, 0.0));
    let bobber = hook(&world, &owner.player);
    let breeze = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 65.0, 3.0),
        &EntityType::BREEZE,
    )));
    world.entities.store(Arc::new(vec![breeze]));
    bobber.entity.set_pos(Vector3::new(0.0, 65.0, 1.0));
    bobber.entity.velocity.store(Vector3::new(0.0, 0.0, 3.0));
    bobber.process_tick(&bobber);
    assert_eq!(bobber.hooked_entity_id.load(Relaxed), -1);
    assert_eq!(bobber.state.load(), HookState::Flying);
    assert!(bobber.entity.pos.load().z < 1.0);
    assert!((bobber.entity.velocity.load().z + 1.38).abs() < 1e-12);
    assert!(!bobber.entity.is_removed());
    assert_eq!(
        bobber.get_owner_id(),
        Some(owner.player.get_entity().entity_id)
    );
    crate::server::fixture_lifecycle::finish().await;
}
