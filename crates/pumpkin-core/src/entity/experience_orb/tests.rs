use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
    world::World,
};
use pumpkin_data::{
    enchantment::Enchantment, entity::EntityType, item::Item, item_stack::ItemStack,
};
use pumpkin_protocol::ser::NetworkReadExt;
use rand::{RngExt, SeedableRng, rngs::StdRng};

fn orb(world: &Arc<World>, value: u32) -> Arc<ExperienceOrbEntity> {
    Arc::new(ExperienceOrbEntity::new(
        Entity::new(
            world.clone(),
            Vector3::new(0.0, 100.0, 0.0),
            &EntityType::EXPERIENCE_ORB,
        ),
        value,
    ))
}

fn same_group(
    world: &Arc<World>,
    first: &ExperienceOrbEntity,
    value: u32,
) -> Arc<ExperienceOrbEntity> {
    let mut entity = Entity::new(
        world.clone(),
        first.entity.pos.load(),
        &EntityType::EXPERIENCE_ORB,
    );
    entity.entity_id = first.entity.entity_id + ORB_GROUPS_PER_AREA;
    Arc::new(ExperienceOrbEntity::new(entity, value))
}

fn matching_random(id: i32) -> StdRng {
    // Award merging is deliberately random in vanilla. Select the existing ID group.
    for seed in 0..10000 {
        let mut rng = StdRng::seed_from_u64(seed);
        if rng.random_range(0..ORB_GROUPS_PER_AREA) == id.rem_euclid(ORB_GROUPS_PER_AREA) {
            return StdRng::seed_from_u64(seed);
        }
    }
    panic!("No seed selected the orb group");
}

#[tokio::test]
async fn award_merges_equal_values_only_in_the_selected_group_and_box() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = Vector3::new(0.0, 100.0, 0.0);
    ExperienceOrbEntity::award_with_random(
        &world,
        pos,
        Vector3::default(),
        7,
        &mut StdRng::seed_from_u64(0),
    );
    let first = world.entities.load_full()[0].clone();
    let first = first
        .cast_any()
        .downcast_ref::<ExperienceOrbEntity>()
        .unwrap();
    first.state.lock().unwrap().age = 150;
    ExperienceOrbEntity::award_with_random(
        &world,
        pos,
        Vector3::default(),
        7,
        &mut matching_random(first.entity.entity_id),
    );
    assert_eq!(world.entities.load().len(), 1);
    assert_eq!(first.state.lock().unwrap().count, 2);
    assert_eq!(first.state.lock().unwrap().age, 0);
    ExperienceOrbEntity::award_with_random(
        &world,
        pos,
        Vector3::default(),
        3,
        &mut matching_random(first.entity.entity_id),
    );
    assert_eq!(world.entities.load().len(), 2);
    ExperienceOrbEntity::award_with_random(
        &world,
        pos.add_raw(2.0, 0.0, 0.0),
        Vector3::default(),
        7,
        &mut matching_random(first.entity.entity_id),
    );
    assert_eq!(world.entities.load().len(), 3);
    // The Java constant 40 partitions IDs, and does not cap represented count.
    first.state.lock().unwrap().count = 40;
    ExperienceOrbEntity::award_with_random(
        &world,
        pos,
        Vector3::default(),
        7,
        &mut matching_random(first.entity.entity_id),
    );
    assert_eq!(first.state.lock().unwrap().count, 41);
    let mut entity = Entity::new(world.clone(), pos, &EntityType::EXPERIENCE_ORB);
    entity.entity_id = i32::MIN;
    let wrapped = ExperienceOrbEntity::new(entity, 7);
    // Java's wrapped difference is 40, so these IDs belong to the same group.
    assert!(wrapped.can_merge(i32::MAX - 39, 7));
}

#[tokio::test]
async fn tick_21_scans_merge_counts_and_keep_the_younger_age() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let first = orb(&world, 7);
    let second = same_group(&world, &first, 7);
    first.state.lock().unwrap().age = 400;
    second.state.lock().unwrap().age = 25;
    second.state.lock().unwrap().count = 3;
    world
        .entities
        .store(Arc::new(vec![first.clone(), second.clone()]));
    first.entity.velocity.store(Vector3::default());
    first.entity.has_no_gravity.store(true, Ordering::Relaxed);
    first.entity.age.store(20, Ordering::Relaxed);
    first.tick(first.as_ref(), &server);
    assert_eq!(world.entities.load().len(), 2);
    first.entity.age.store(21, Ordering::Relaxed);
    first.tick(first.as_ref(), &server);
    assert_eq!(world.entities.load().len(), 1);
    assert_eq!(first.state.lock().unwrap().count, 4);
    assert_eq!(first.state.lock().unwrap().age, 26);
    assert!(second.entity.is_removed());
}

#[tokio::test]
async fn reciprocal_parallel_merge_scans_do_not_lose_counts() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let first = orb(&world, 7);
    let second = same_group(&world, &first, 7);
    first.state.lock().unwrap().count = 9;
    second.state.lock().unwrap().count = 14;
    world
        .entities
        .store(Arc::new(vec![first.clone(), second.clone()]));
    rayon::join(|| first.scan_for_merges(), || second.scan_for_merges());
    assert_eq!(world.entities.load().len(), 1);
    let survivor = world.entities.load_full()[0].clone();
    assert_eq!(
        survivor
            .cast_any()
            .downcast_ref::<ExperienceOrbEntity>()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .count,
        23
    );
}

#[tokio::test]
async fn following_adds_vanilla_acceleration_and_keeps_dead_targets_until_repick() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let first = TestPlayer::new(&world);
    let second = TestPlayer::new(&world);
    let orb = orb(&world, 1);
    let eye_offset = first.player.get_entity().get_eye_height() / 2.0;
    first
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 100.0 - eye_offset, 0.0));
    second
        .player
        .get_entity()
        .set_pos(Vector3::new(-4.0, 100.0 - eye_offset, 0.0));
    world
        .players
        .store(Arc::new(vec![first.player.clone(), second.player.clone()]));
    orb.entity.velocity.store(Vector3::new(0.01, 0.0, 0.0));
    assert!(orb.follow_nearby_player());
    // Four blocks away: (1 - 4/8)^2 * 0.1 = 0.025, added to 0.01.
    assert!((orb.entity.velocity.load().x - 0.035).abs() < 1.0e-12);
    assert!(orb.entity.velocity.load().y.abs() < 1.0e-12);
    first.player.living_entity.health.store(0.0);
    assert!(orb.follow_nearby_player());
    assert!((orb.entity.velocity.load().x - 0.060).abs() < 1.0e-12);
    first
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Spectator);
    assert!(orb.follow_nearby_player());
    assert!((orb.entity.velocity.load().x - 0.035).abs() < 1.0e-12);
    // A fresh selection stops at the nearest non-spectator, even when that player is dead.
    first
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Survival);
    first
        .player
        .get_entity()
        .set_pos(Vector3::new(1.0, 100.0, 0.0));
    second
        .player
        .get_entity()
        .set_pos(Vector3::new(-10.0, 100.0, 0.0));
    assert!(!orb.follow_nearby_player());
    second
        .player
        .get_entity()
        .set_pos(Vector3::new(-4.0, 100.0, 0.0));
    assert!(!orb.follow_nearby_player());
}

#[tokio::test]
async fn pickup_consumes_one_count_every_two_player_ticks_and_sends_animation() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut player = TestPlayer::new(&world);
    let orb = orb(&world, 7);
    orb.state.lock().unwrap().count = 2;
    player.take_packets();
    orb.on_player_collision(&player.player);
    assert_eq!(orb.state.lock().unwrap().count, 1);
    assert!(!orb.entity.is_removed());
    assert_eq!(
        player
            .player
            .experience_pick_up_delay
            .load(Ordering::Relaxed),
        2
    );
    let packets = player.take_packets();
    let animation = packets
        .iter()
        .find_map(|packet| {
            let mut bytes = packet.as_ref();
            (bytes.get_var_int().unwrap().0
                == pumpkin_data::packet::clientbound::play::TAKE_ITEM_ENTITY.0)
                .then_some(bytes)
        })
        .unwrap();
    let mut animation = animation;
    assert_eq!(animation.get_var_int().unwrap().0, orb.entity.entity_id);
    assert_eq!(
        animation.get_var_int().unwrap().0,
        player.player.get_entity().entity_id
    );
    assert_eq!(animation.get_var_int().unwrap().0, 1);
    orb.on_player_collision(&player.player);
    assert_eq!(orb.state.lock().unwrap().count, 1);
    player.player.tick(&server);
    orb.on_player_collision(&player.player);
    assert_eq!(orb.state.lock().unwrap().count, 1);
    player.player.tick(&server);
    orb.on_player_collision(&player.player);
    assert_eq!(orb.state.lock().unwrap().count, 0);
    assert!(orb.entity.is_removed());
    assert_eq!(player.player.experience_level.load(Ordering::Relaxed), 1);
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 7);
}

#[tokio::test]
async fn expanded_player_collection_skips_spectators_and_dead_players() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let spectator = TestPlayer::new(&world);
    let dead = TestPlayer::new(&world);
    let alive = TestPlayer::new(&world);
    spectator
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Spectator);
    dead.player.living_entity.health.store(0.0);
    for player in [&spectator, &dead, &alive] {
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(1.2, 100.0, 0.0));
    }
    let orb = orb(&world, 3);
    assert!(
        !orb.entity
            .bounding_box
            .load()
            .intersects(&alive.player.get_entity().bounding_box.load())
    );
    collect_nearby_orbs(
        &[
            spectator.player.clone(),
            dead.player.clone(),
            alive.player.clone(),
        ],
        &[orb.clone() as Arc<dyn EntityBase>],
    );
    assert_eq!(alive.player.experience_points.load(Ordering::Relaxed), 3);
    assert_eq!(
        spectator.player.experience_points.load(Ordering::Relaxed),
        0
    );
    assert_eq!(dead.player.experience_points.load(Ordering::Relaxed), 0);
    assert!(orb.entity.is_removed());
}

#[tokio::test]
async fn saved_high_value_orbs_restore_health_age_count_and_client_metadata() {
    use pumpkin_nbt::{Nbt, deserializer::NbtReadHelperJava};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let original = orb(&world, 2477);
    *original.state.lock().unwrap() = OrbState {
        health: 3,
        age: 3210,
        count: 12,
    };
    let mut nbt = NbtCompound::new();
    original.write_nbt(&mut nbt);
    assert_eq!(nbt.get_short("Value"), Some(2477));
    let bytes = Nbt::new(String::new(), nbt).write_unnamed();
    let nbt = Nbt::read_unnamed(&mut NbtReadHelperJava::new(std::io::Cursor::new(
        bytes.as_ref(),
    )))
    .unwrap();
    let restored = crate::entity::r#type::from_type(
        &EntityType::EXPERIENCE_ORB,
        Vector3::default(),
        &world,
        uuid::Uuid::new_v4(),
    );
    restored.read_nbt_non_mut(&nbt.root_tag);
    let restored = restored
        .cast_any()
        .downcast_ref::<ExperienceOrbEntity>()
        .unwrap();
    assert_eq!(restored.get_value(), 2477);
    let state = restored.state.lock().unwrap();
    assert_eq!((state.health, state.age, state.count), (3, 3210, 12));
    drop(state);
    let bytes = restored
        .entity
        .synched_data
        .get_non_default_values_for_version(&pumpkin_data::packet::CURRENT_MC_VERSION)
        .unwrap();
    // DATA_VALUE is tracked index 8 and serializer INT (VarInt), followed by 2477.
    assert!(bytes.windows(4).any(|bytes| bytes == [8, 1, 0xad, 0x13]));
    let mut invalid = NbtCompound::new();
    invalid.put_int("Count", -3);
    restored.read_custom_nbt(&invalid);
    assert_eq!(restored.state.lock().unwrap().count, 1);
    assert_eq!(restored.get_value(), 0);
    // TagValueInput accepts numeric tags, including integer Value from summon commands.
    let mut numeric = NbtCompound::new();
    numeric.put_int("Health", 3);
    numeric.put_float("Age", 3210.9);
    numeric.put_int("Value", 2477);
    numeric.put_short("Count", 12);
    restored.read_custom_nbt(&numeric);
    assert_eq!(restored.get_value(), 2477);
    let state = restored.state.lock().unwrap();
    assert_eq!((state.health, state.age, state.count), (3, 3210, 12));
    drop(state);
    numeric.put_long("Count", 4_294_967_308);
    restored.read_custom_nbt(&numeric);
    assert_eq!(restored.state.lock().unwrap().count, 12);
}

#[tokio::test]
async fn one_xp_repairs_two_equipped_items_missing_one_durability() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::MENDING, 1);
    sword.set_damage(1);
    player.player.inventory.set_slot(0, sword.clone());
    player.player.inventory.set_slot(
        pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT,
        sword,
    );
    // Each one-point repair spends 1 * 1 / 2 = 0 XP in vanilla; the point remains.
    assert_eq!(player.player.apply_mending_from_xp(1), 1);
    assert_eq!(player.player.inventory.get_slot(0).get_damage(), 0);
    assert_eq!(player.player.inventory.get_slot(40).get_damage(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_pickup_and_mending_preserve_the_orb_and_damaged_equipment() {
    use crate::plugin::api::events::{
        entity::entity_pickup_item::EntityPickupItemEvent,
        player::player_item_mend::PlayerItemMendEvent,
    };
    use crate::plugin::{BoxFuture, EventHandler, EventPriority, Payload};
    struct Cancel;
    impl EventHandler<EntityPickupItemEvent> for Cancel {
        fn handle_blocking<'a>(
            &'a self,
            _server: &'a Arc<Server>,
            event: &'a mut EntityPickupItemEvent,
        ) -> BoxFuture<'a, ()> {
            event.cancelled = true;
            Box::pin(async {})
        }
    }
    struct CancelMend;
    impl EventHandler<PlayerItemMendEvent> for CancelMend {
        fn handle_blocking<'a>(
            &'a self,
            _server: &'a Arc<Server>,
            event: &'a mut PlayerItemMendEvent,
        ) -> BoxFuture<'a, ()> {
            event.cancelled = true;
            Box::pin(async {})
        }
    }
    fn register<E: Payload + Send + Sync + 'static, H: EventHandler<E> + 'static>(
        server: &Server,
        handler: H,
    ) {
        server
            .plugin_manager
            .register::<E, _>(Arc::new(handler), EventPriority::Normal, true);
    }
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let orb = orb(&world, 7);
    register::<EntityPickupItemEvent, _>(&server, Cancel);
    orb.on_player_collision(&player.player);
    assert_eq!(orb.state.lock().unwrap().count, 1);
    assert!(!orb.entity.is_removed());
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 0);
    register::<PlayerItemMendEvent, _>(&server, CancelMend);
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::MENDING, 1);
    sword.set_damage(1);
    player.player.inventory.set_slot(0, sword);
    assert_eq!(player.player.apply_mending_from_xp(1), 1);
    assert_eq!(player.player.inventory.get_slot(0).get_damage(), 1);
}

#[tokio::test]
async fn directed_awards_launch_outward_from_the_supplied_position() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let entity = Entity::new(
        world,
        Vector3::new(0.0, 100.0, 0.0),
        &EntityType::EXPERIENCE_ORB,
    );
    let orb = ExperienceOrbEntity::new_with_direction(
        entity,
        Vector3::new(1.0, 0.0, 0.0),
        7,
        &mut StdRng::seed_from_u64(0),
    );
    assert!((orb.entity.pos.load().x - 0.25).abs() < 1.0e-12);
    assert!(orb.entity.velocity.load().x >= 0.0);
    assert!(orb.entity.velocity.load().length_squared() > 0.0);
}

#[tokio::test]
async fn health_and_lifetime_discard_at_vanilla_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let hurt = orb(&world, 1);
    assert!(hurt.damage(hurt.as_ref(), 1.5, DamageType::IN_FIRE));
    assert_eq!(hurt.state.lock().unwrap().health, 3);
    hurt.entity.invulnerable.store(true, Ordering::Relaxed);
    assert!(!hurt.damage(hurt.as_ref(), 10.0, DamageType::IN_FIRE));
    hurt.entity.invulnerable.store(false, Ordering::Relaxed);
    assert!(hurt.damage(hurt.as_ref(), 3.0, DamageType::IN_FIRE));
    assert!(hurt.entity.is_removed());
    let old = orb(&world, 1);
    old.state.lock().unwrap().age = 5999;
    old.tick_age();
    assert!(old.entity.is_removed());
}

#[test]
fn underwater_buoyancy_promotes_java_floats_and_caps_upward_speed() {
    let velocity = ExperienceOrbEntity::underwater_movement(Vector3::new(0.2, -0.1, -0.2));
    assert!((velocity.x - 0.198_000_001_907_348_64).abs() < 1.0e-15);
    assert!((velocity.y + 0.099_499_999_976_251_28).abs() < 1.0e-15);
    let velocity = ExperienceOrbEntity::underwater_movement(Vector3::new(0.0, 0.0598, 0.0));
    assert!((velocity.y - 0.059_999_998_658_895_49).abs() < 1.0e-15);
}
