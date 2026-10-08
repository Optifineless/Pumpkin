use super::*;
use crate::entity::{
    living::test_support::armor_test_world, projectile_deflection::ProjectileDeflectionType,
};
use pumpkin_nbt::compound::NbtCompound;

#[tokio::test]
async fn owner_uuid_survives_reload_and_owner_grace_depends_on_collision_not_age() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        Some(owner.entity.entity_id),
    );
    arrow.pickup.store(arrow::ArrowPickup::Allowed);
    *arrow.weapon.write().unwrap() = Some(ItemStack::new(1, &pumpkin_data::item::Item::BOW));
    arrow.entity.age.store(100, Ordering::Relaxed);
    arrow.projectile.tick(&arrow.entity);
    let owner_dyn: Arc<dyn EntityBase> = owner.clone();
    assert!(!arrow.projectile.can_hit(&arrow.entity, &owner_dyn));
    arrow.entity.set_pos(Vector3::new(10.0, 0.0, 0.0));
    arrow.projectile.tick(&arrow.entity);
    assert!(arrow.projectile.can_hit(&arrow.entity, &owner_dyn));
    let mut nbt = NbtCompound::new();
    EntityBase::write_nbt(&arrow, &mut nbt);
    assert_eq!(nbt.get_uuid("Owner"), Some(owner.entity.entity_uuid));
    let restored_owner = Arc::new(LivingEntity::new(Entity::from_uuid(
        owner.entity.entity_uuid,
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    assert_ne!(owner.entity.entity_id, restored_owner.entity.entity_id);
    world.entities.store(Arc::new(vec![restored_owner.clone()]));
    let restored = arrow::ArrowEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::ARROW),
        None,
    );
    EntityBase::read_nbt_non_mut(&restored, &nbt);
    assert_eq!(restored.pickup.load(), arrow::ArrowPickup::Allowed);
    assert_eq!(
        restored.get_weapon_item().unwrap().item,
        &pumpkin_data::item::Item::BOW
    );
    assert_eq!(
        restored.get_owner_id(),
        Some(restored_owner.entity.entity_id)
    );
    let restored_owner: Arc<dyn EntityBase> = restored_owner;
    assert!(
        restored
            .projectile
            .can_hit(&restored.entity, &restored_owner)
    );
}

#[tokio::test]
async fn deflection_changes_velocity_owner_and_hurting_acceleration() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let fireball = fireball::FireballEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::FIREBALL,
    ));
    fireball
        .get_entity()
        .velocity
        .store(Vector3::new(2.0, -3.0, 4.0));
    assert!(deflection::deflect(
        &fireball,
        ProjectileDeflectionType::Simple,
        Some(owner.as_ref()),
        Some(owner.as_ref()),
        false,
        Vector3::new(0.2, 0.2, 0.2)
    ));
    let velocity = fireball.get_entity().velocity.load();
    assert!((velocity.x + 0.2).abs() < 1.0e-12);
    assert!((velocity.y - 0.3).abs() < 1.0e-12);
    assert!((velocity.z + 0.4).abs() < 1.0e-12);
    assert_eq!(fireball.get_owner_id(), Some(owner.entity.entity_id));
    assert_eq!(fireball.get_acceleration_power(), 0.05);
    assert!(deflection::deflect(
        &fireball,
        ProjectileDeflectionType::Redirected,
        Some(owner.as_ref()),
        Some(owner.as_ref()),
        true,
        Vector3::new(1.0, 1.0, 1.0)
    ));
    assert_eq!(fireball.get_acceleration_power(), 0.1);
    assert_eq!(
        fireball.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, 1.0)
    );
    owner.entity.velocity.store(Vector3::new(3.0, 0.0, 4.0));
    deflection::deflect(
        &fireball,
        ProjectileDeflectionType::TransferVelocityDirection,
        Some(owner.as_ref()),
        None,
        true,
        Vector3::new(0.5, 2.0, 3.0),
    );
    let velocity = fireball.get_entity().velocity.load();
    assert!((velocity.x - 0.3).abs() < 1.0e-12);
    assert!((velocity.z - 2.4).abs() < 1.0e-12);
    assert_eq!(fireball.get_owner_id(), None);
    let wind = wind_charge::WindChargeEntity::new_normal(ThrownItemEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::WIND_CHARGE),
        &owner.entity,
        0.0,
    ));
    assert!(!deflection::deflect(
        &wind,
        ProjectileDeflectionType::Redirected,
        Some(owner.as_ref()),
        Some(owner.as_ref()),
        true,
        Vector3::new(1.0, 1.0, 1.0)
    ));
    wind.deflect_cooldown().unwrap().store(0, Ordering::Relaxed);
    assert!(deflection::deflect(
        &wind,
        ProjectileDeflectionType::Redirected,
        Some(owner.as_ref()),
        Some(owner.as_ref()),
        true,
        Vector3::new(1.0, 1.0, 1.0)
    ));
}

#[test]
fn splash_geometry_and_duration_keep_the_vanilla_boundary_rules() {
    use super::potion_effects::{box_distance_squared, scaled_duration};
    let bottle = BoundingBox::new(
        Vector3::new(-0.125, 0.0, -0.125),
        Vector3::new(0.125, 0.25, 0.125),
    );
    let target = BoundingBox::new(Vector3::new(3.7, -1.0, -0.3), Vector3::new(4.3, 0.8, 0.3));
    assert!((box_distance_squared(bottle, target) - 12.780625).abs() < 0.000001);
    assert_eq!(scaled_duration(101, 0.5), 51);
    assert_eq!(scaled_duration(-1, 0.1), -1);
    assert_eq!(scaled_duration(0, 0.5), 0);
    assert_eq!(super::potion_effects::with_scaled_duration(103, 0.25), 25);
    assert_eq!(super::potion_effects::with_scaled_duration(1, 0.125), 1);
    assert_eq!(super::potion_effects::with_scaled_duration(-1, 0.125), -1);
}

#[tokio::test]
async fn breeze_deflection_consumes_the_collision_without_discarding_the_projectile() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let breeze: Arc<dyn EntityBase> = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 65.0, 3.0),
        &EntityType::BREEZE,
    )));
    let snowball = snowball::SnowballEntity::new(Entity::new(
        world,
        Vector3::new(0.0, 65.0, 0.0),
        &EntityType::SNOWBALL,
    ));
    snowball
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 2.0));
    let hit = || ProjectileHit::Entity {
        entity: breeze.clone(),
        hit_pos: Vector3::new(0.0, 65.0, 3.0),
        normal: Vector3::new(0.0, 0.0, -1.0),
    };
    snowball.thrown.hit_target(&snowball, hit());
    assert!(!snowball.get_entity().is_removed());
    assert_eq!(
        snowball.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, -1.0)
    );
    snowball.thrown.hit_target(&snowball, hit());
    assert_eq!(
        snowball.get_entity().velocity.load(),
        Vector3::new(0.0, 0.0, -1.0)
    );
}

#[tokio::test]
async fn water_potions_douse_candles_campfires_and_burning_living_targets() {
    use pumpkin_data::{
        Block,
        block_properties::{CampfireLikeProperties, CandleLikeProperties},
        data_component_impl::PotionContentsImpl,
    };
    use pumpkin_util::math::vector2::Vector2;
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
    let mut candle = CandleLikeProperties::from_state_id(Block::CANDLE.default_state.id);
    candle.lit = true;
    chunk.set_block_absolute_y(8, 65, 8, candle.to_state_id(&Block::CANDLE));
    let mut campfire = CampfireLikeProperties::from_state_id(Block::CAMPFIRE.default_state.id);
    campfire.lit = true;
    chunk.set_block_absolute_y(9, 66, 8, campfire.to_state_id(&Block::CAMPFIRE));
    world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
    let target = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 65.0, 9.5),
        &EntityType::COW,
    )));
    target.entity.set_on_fire_for(5.0);
    world.entities.store(Arc::new(vec![target.clone()]));
    let potion = splash_potion::SplashPotionEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 66.0, 8.5),
        &EntityType::SPLASH_POTION,
    ));
    let mut stack = ItemStack::new(1, &pumpkin_data::item::Item::SPLASH_POTION);
    stack.set_data_component(PotionContentsImpl {
        potion_id: Some(pumpkin_data::potion::Potion::WATER.id.into()),
        custom_color: None,
        custom_effects: vec![],
        custom_name: None,
    });
    potion.set_item_stack(stack);
    potion.on_hit(ProjectileHit::Block {
        pos: BlockPos::new(8, 65, 8),
        world_border: false,
        face: BlockDirection::Up,
        hit_pos: Vector3::new(8.5, 66.0, 8.5),
        normal: Vector3::new(0.0, 1.0, 0.0),
    });
    assert!(!target.entity.is_on_fire());
    assert!(
        !CandleLikeProperties::from_state_id(world.get_block_state(&BlockPos::new(8, 65, 8)).id)
            .lit
    );
    assert!(
        !CampfireLikeProperties::from_state_id(world.get_block_state(&BlockPos::new(9, 66, 8)).id)
            .lit
    );
}

#[tokio::test]
async fn arrows_reverse_at_the_world_border_with_one_tenth_speed() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    world.worldborder.lock().unwrap().new_diameter = 10.0;
    let start = Vector3::new(4.0, 65.0, 0.0);
    let movement = Vector3::new(2.0, 0.0, 0.0);
    let arrow = arrow::ArrowEntity::new(Entity::new(world, start, &EntityType::ARROW), None);
    arrow.entity.velocity.store(movement);
    arrow.step_move_and_hit(&arrow, start, start + movement, movement);
    assert_eq!(arrow.entity.velocity.load(), Vector3::new(-0.2, 0.0, 0.0));
    assert!(arrow.entity.pos.load().x < 5.0);
    assert!(!arrow.entity.is_removed());
    assert!(!arrow.in_ground.load(Ordering::Relaxed));
}
