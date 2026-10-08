use super::*;
use crate::net::java::combat_test_support::TestPlayer;
use crate::{
    server::combat_test_support::{server, world},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_data::{enchantment::Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_inventory::slot::Slot;
use pumpkin_util::math::vector2::Vector2;

fn stationary_orb(
    world: &Arc<crate::world::World>,
    pos: Vector3<f64>,
    value: i32,
) -> Arc<ExperienceOrbEntity> {
    let entity = Entity::new(world.clone(), pos, &EntityType::EXPERIENCE_ORB);
    entity.has_no_gravity.store(true, Ordering::Relaxed);
    let orb = Arc::new(ExperienceOrbEntity::new_empty(entity));
    orb.set_value(value);
    orb
}

#[tokio::test]
async fn single_rewards_preserve_raw_value_and_do_not_merge_on_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = Vector3::new(0.0, 100.0, 0.0);
    for _ in 0..2 {
        ExperienceOrbEntity::spawn_single(&world, pos, 5);
    }
    let entities = world.entities.load();
    assert_eq!(entities.len(), 2);
    for entity in entities.iter() {
        let orb = entity
            .cast_any()
            .downcast_ref::<ExperienceOrbEntity>()
            .unwrap();
        assert_eq!(orb.get_value(), 5);
        assert_eq!(orb.entity.pos.load(), pos);
        assert!(orb.entity.velocity.load().length_squared() > 0.0);
        assert_eq!(orb.state.lock().unwrap().count, 1);
    }
}

struct CollisionProbe {
    entity: Entity,
    touches: AtomicI32,
}
impl EntityBase for CollisionProbe {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn on_player_collision(&self, _player: &Arc<Player>) {
        self.touches.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn world_tick_collects_expanded_orbs_and_skips_generic_orb_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = TestPlayer::new(&world);
    let pos = Vector3::new(8.0, 100.0, 8.0);
    player
        .player
        .get_entity()
        .set_pos(pos.add_raw(1.2, 0.0, 0.0));
    let orb = stationary_orb(&world, pos, 3);
    let probe = Arc::new(CollisionProbe {
        entity: Entity::new(
            world.clone(),
            player.player.position(),
            &EntityType::EXPERIENCE_ORB,
        ),
        touches: AtomicI32::new(0),
    });
    assert!(
        !orb.entity
            .bounding_box
            .load()
            .intersects(&player.player.get_entity().bounding_box.load())
    );
    world
        .entities
        .store(Arc::new(vec![orb.clone(), probe.clone()]));
    world.tick(&server);
    assert_eq!(orb.entity.age.load(Ordering::Relaxed), 1);
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 3);
    assert!(orb.entity.is_removed());
    assert_eq!(probe.touches.load(Ordering::Relaxed), 0);
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
async fn seven_xp_repairs_three_damage_spends_one_and_gives_six() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::MENDING, 1);
    sword.set_damage(3);
    player.player.inventory.set_slot(0, sword);
    let orb = stationary_orb(&world, player.player.position(), 7);
    orb.on_player_collision(&player.player);
    assert_eq!(player.player.inventory.get_slot(0).get_damage(), 0);
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 6);
    assert_eq!(player.player.experience_level.load(Ordering::Relaxed), 0);
    assert!(orb.entity.is_removed());
}

#[tokio::test]
async fn summon_factory_defaults_to_zero_value_and_no_motion() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = Vector3::new(8.0, 100.0, 8.0);
    let entity = crate::entity::r#type::from_type(
        &EntityType::EXPERIENCE_ORB,
        pos,
        &world,
        uuid::Uuid::new_v4(),
    );
    entity.init_data_tracker();
    let orb = entity
        .cast_any()
        .downcast_ref::<ExperienceOrbEntity>()
        .unwrap();
    assert_eq!(orb.get_value(), 0);
    assert_eq!(orb.entity.velocity.load(), Vector3::default());
    assert_eq!(orb.entity.pos.load(), pos);
    let player = TestPlayer::new(&world);
    orb.on_player_collision(&player.player);
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn death_loot_award_merges_through_the_real_death_spawn_site() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 100.0, 8.0));
    player.player.experience_level.store(1, Ordering::Relaxed);
    // Cover all random award groups so this checks the spawn site without a probabilistic assertion.
    let mut orbs = Vec::new();
    for group in 0..ORB_GROUPS_PER_AREA {
        let mut entity = Entity::new(
            world.clone(),
            player.player.position(),
            &EntityType::EXPERIENCE_ORB,
        );
        entity.entity_id = 1_000_000 + group;
        let orb = Arc::new(ExperienceOrbEntity::new_empty(entity));
        orb.set_value(7);
        orb.state.lock().unwrap().age = 100;
        orbs.push(orb);
    }
    world.entities.store(Arc::new(
        orbs.iter()
            .cloned()
            .map(|orb| orb as Arc<dyn EntityBase>)
            .collect(),
    ));
    player.player.living_entity.health.store(0.0);
    player
        .player
        .living_entity
        .on_death(DamageType::GENERIC_KILL, None, None);
    assert!(player.player.living_entity.dead.load(Ordering::Relaxed));
    assert_eq!(world.entities.load().len(), 40);
    assert_eq!(
        orbs.iter()
            .map(|orb| orb.state.lock().unwrap().count)
            .sum::<i32>(),
        41
    );
    assert_eq!(
        orbs.iter()
            .filter(|orb| orb.state.lock().unwrap().age == 0)
            .count(),
        1
    );
}

#[tokio::test]
async fn collection_ignores_a_player_who_changed_world_after_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    let orb = stationary_orb(&world, player.player.position(), 3);
    let other_dir = tempfile::tempdir().unwrap();
    let other = crate::server::combat_test_support::world(&server, other_dir.path());
    player.player.get_entity().world.store(other);
    let entity: Arc<dyn EntityBase> = orb.clone();
    collect_nearby_orbs(
        std::slice::from_ref(&player.player),
        std::slice::from_ref(&entity),
    );
    assert!(!orb.entity.is_removed());
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn tiny_nonzero_directions_normalize_for_launch_and_following() {
    use rand::{SeedableRng, rngs::StdRng};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = Vector3::new(8.0, 100.0, 8.0);
    let orb = ExperienceOrbEntity::new_with_direction(
        Entity::new(world.clone(), pos, &EntityType::EXPERIENCE_ORB),
        Vector3::new(5.0e-5, 0.0, 0.0),
        3,
        &mut StdRng::seed_from_u64(0),
    );
    assert_eq!(orb.entity.pos.load().x, 8.25);
    orb.entity.set_pos(pos);
    orb.entity.velocity.store(Vector3::default());
    let player = TestPlayer::new(&world);
    player.player.get_entity().set_pos(pos.add_raw(
        5.0e-5,
        -player.player.get_entity().get_eye_height() / 2.0,
        0.0,
    ));
    assert!(orb.follow_nearby_player());
    assert!(orb.entity.velocity.load().x > 0.099);
}

#[tokio::test]
async fn unsticking_nan_position_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let orb = stationary_orb(&world, Vector3::new(f64::NAN, 100.0, 0.0), 1);
    orb.unstuck_if_possible(0.5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn furnace_output_awards_each_recipe_separately() {
    use crate::block::entities::furnace::FurnaceBlockEntity;
    use pumpkin_inventory::furnace_like::furnace_like_slot::FurnaceOutputSlot;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 100.0, 8.0));
    let furnace = Arc::new(FurnaceBlockEntity::new(
        pumpkin_util::math::position::BlockPos::new(0, 64, 0),
    ));
    // AbstractFurnaceBlockEntity.getRecipesToAwardAndPopExperience awards each recipe on its
    // own: two 0.7 XP recipes smelted ten times give two awards of 7, never one award of 14
    // (which would split into an 11 and a 3).
    let mut recipes = furnace.recipes_used.lock().unwrap();
    recipes.insert("minecraft:iron_ingot_from_smelting_iron_ore".into(), 10);
    recipes.insert("minecraft:copper_ingot_from_smelting_copper_ore".into(), 10);
    drop(recipes);
    let slot = FurnaceOutputSlot::new(furnace.clone(), furnace);
    slot.on_take_item(
        player.player.as_ref(),
        &ItemStack::new(10, &Item::IRON_INGOT),
    );
    let orbs = world.entities.load_full();
    let mut total = 0;
    for entity in orbs.iter() {
        let orb = entity
            .cast_any()
            .downcast_ref::<ExperienceOrbEntity>()
            .unwrap();
        assert_eq!(orb.get_value(), 7);
        total += orb.get_value() * orb.state.lock().unwrap().count;
    }
    assert_eq!(total, 14);
}

#[tokio::test]
async fn furnace_output_spawns_orbs_at_player_and_allows_mending() {
    use crate::block::entities::furnace::FurnaceBlockEntity;
    use pumpkin_inventory::furnace_like::furnace_like_slot::FurnaceOutputSlot;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world);
    player
        .player
        .get_entity()
        .set_pos(Vector3::new(8.0, 100.0, 8.0));
    let furnace = Arc::new(FurnaceBlockEntity::new(
        pumpkin_util::math::position::BlockPos::new(0, 64, 0),
    ));
    furnace
        .recipes_used
        .lock()
        .unwrap()
        .insert("minecraft:iron_ingot_from_smelting_iron_ore".into(), 10);
    let slot = FurnaceOutputSlot::new(furnace.clone(), furnace);
    let mut sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    sword.add_enchantment(&Enchantment::MENDING, 1);
    sword.set_damage(3);
    player.player.inventory.set_slot(0, sword);
    slot.on_take_item(
        player.player.as_ref(),
        &ItemStack::new(10, &Item::IRON_INGOT),
    );
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 0);
    let orbs = world.entities.load_full();
    assert_eq!(orbs.len(), 1);
    let orb = orbs[0]
        .cast_any()
        .downcast_ref::<ExperienceOrbEntity>()
        .unwrap();
    assert_eq!(orb.get_value(), 7);
    assert_eq!(orb.entity.pos.load(), player.player.position());
    orb.on_player_collision(&player.player);
    assert_eq!(player.player.inventory.get_slot(0).get_damage(), 0);
    assert_eq!(player.player.experience_points.load(Ordering::Relaxed), 6);
}

#[tokio::test]
async fn breaking_furnaces_and_xp_blocks_awards_at_block_centers() {
    use crate::block::entities::{BlockEntity, furnace::FurnaceBlockEntity};
    use pumpkin_util::math::position::BlockPos;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let pos = BlockPos::new(8, 100, 8);
    let furnace = Arc::new(FurnaceBlockEntity::new(pos));
    furnace
        .recipes_used
        .lock()
        .unwrap()
        .insert("minecraft:iron_ingot_from_smelting_iron_ore".into(), 10);
    furnace.on_block_replaced(&world, &pos);
    assert_eq!(
        world.entities.load()[0].get_entity().pos.load(),
        pos.to_centered_f64()
    );
    world.entities.store(Arc::new(Vec::new()));
    // Diamond ore has guaranteed positive XP in the extracted block data.
    crate::block::drop_loot(
        &world,
        &Block::DIAMOND_ORE,
        &pos,
        true,
        &crate::world::loot::LootContextParameters::default(),
    );
    let entities = world.entities.load();
    let orbs: Vec<_> = entities
        .iter()
        .filter_map(|entity| entity.cast_any().downcast_ref::<ExperienceOrbEntity>())
        .collect();
    assert!(!orbs.is_empty());
    for orb in orbs {
        assert_eq!(orb.get_entity().pos.load(), pos.to_centered_f64());
    }
}

struct TypeProbe {
    entity: Entity,
    casts: AtomicI32,
}
impl EntityBase for TypeProbe {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self.casts.fetch_add(1, Ordering::Relaxed);
        self
    }
}

#[tokio::test]
async fn world_tick_filters_non_orbs_once_for_all_merge_scans() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let pos = Vector3::new(8.0, 100.0, 8.0);
    let probe = Arc::new(TypeProbe {
        entity: Entity::new(world.clone(), pos, &EntityType::ARMOR_STAND),
        casts: AtomicI32::new(0),
    });
    let mut entities: Vec<Arc<dyn EntityBase>> = vec![probe.clone()];
    for value in 1..=40 {
        entities.push(stationary_orb(&world, pos, value));
    }
    world.entities.store(Arc::new(entities));
    world.tick(&server);
    assert_eq!(probe.casts.load(Ordering::Relaxed), 1);
    assert!(world.ticking_experience_orbs.load().is_none());
    world.level.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "manual debug-profile timing comparison"]
#[expect(clippy::print_stdout, reason = "Requested benchmark prints wall time")]
async fn debug_world_tick_2000_entities_200_orbs_100_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    world
        .forced_chunks
        .lock()
        .unwrap()
        .insert(Vector2::new(0, 0));
    let mut entities: Vec<Arc<dyn EntityBase>> = Vec::with_capacity(2200);
    for index in 0..2000 {
        entities.push(Arc::new(Entity::new(
            world.clone(),
            Vector3::new(8.0, 70.0 + f64::from(index % 100), 8.0),
            &EntityType::ARMOR_STAND,
        )));
    }
    let mut orbs = Vec::new();
    for index in 0..200 {
        let entity = Entity::new(
            world.clone(),
            Vector3::new(8.0, 70.0 + f64::from(index) * 0.8, 8.0),
            &EntityType::EXPERIENCE_ORB,
        );
        entity.has_no_gravity.store(true, Ordering::Relaxed);
        let orb = Arc::new(ExperienceOrbEntity::new_empty(entity));
        orb.set_value(index + 1);
        entities.push(orb.clone());
        orbs.push(orb);
    }
    world.entities.store(Arc::new(entities));
    let started = std::time::Instant::now();
    for _ in 0..100 {
        world.tick(&server);
    }
    println!(
        "debug profile: 2,000 entities + 200 orbs, 100 World::tick calls: {:?}",
        started.elapsed()
    );
    assert!(
        orbs.iter()
            .all(|orb| orb.entity.age.load(Ordering::Relaxed) == 100)
    );
    world.level.shutdown().await.unwrap();
}
