use super::*;
use crate::{
    entity::living::test_support::armor_test_world,
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support::{server, world},
};
use pumpkin_data::{Enchantment, data_component_impl::PotionContentsImpl, item::Item};
use pumpkin_nbt::NbtCompound;
use std::sync::Mutex;

#[tokio::test]
async fn healing_splash_outer_range_and_dead_targets_are_harmless() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let target = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(4.5, 64.0, 0.0),
        &EntityType::COW,
    )));
    let potion = splash_potion::SplashPotionEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 64.0, 0.0),
        &EntityType::SPLASH_POTION,
    ));
    let mut box_ = target.entity.bounding_box.load();
    box_.min.x = potion.get_entity().bounding_box.load().max.x + 3.75;
    box_.max.x = box_.min.x + 0.6;
    target.entity.bounding_box.store(box_);
    world.entities.store(Arc::new(vec![target.clone()]));
    let mut stack = ItemStack::new(1, &Item::SPLASH_POTION);
    stack.set_data_component(PotionContentsImpl {
        potion_id: Some(pumpkin_data::potion::Potion::HEALING.id.into()),
        custom_color: None,
        custom_effects: vec![],
        custom_name: None,
    });
    potion.set_item_stack(stack);
    target.set_health(5.0);
    let splash = || {
        potion.on_hit(ProjectileHit::Block {
            pos: BlockPos::new(0, 64, 0),
            world_border: false,
            face: BlockDirection::Up,
            hit_pos: potion.get_entity().pos.load(),
            normal: Vector3::default(),
        });
    };
    splash();
    assert_eq!(target.health.load(), 5.0);
    // A direct full-potency hit must not resurrect a target whose death is already committed.
    target.dead.store(true, Ordering::Relaxed);
    target.entity.set_pos(potion.get_entity().pos.load());
    splash();
    assert_eq!(target.health.load(), 5.0);
}

#[tokio::test]
async fn saved_weapon_preserves_arrow_hit_damage_and_punch_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let mut bow = ItemStack::new(1, &Item::BOW);
    bow.add_enchantment(&Enchantment::POWER, 1);
    bow.add_enchantment(&Enchantment::PUNCH, 2);
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        None,
    );
    *arrow.weapon.write().unwrap() = Some(bow);
    arrow.entity.velocity.store(Vector3::new(0.0, 0.0, 2.0));
    let mut saved = NbtCompound::new();
    EntityBase::write_nbt(&arrow, &mut saved);
    let restored = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        None,
    );
    EntityBase::read_nbt_non_mut(&restored, &saved);
    for projectile in [&restored, &arrow] {
        let target = Arc::new(test_support::Receiver {
            living: LivingEntity::new(Entity::new(
                world.clone(),
                Vector3::default(),
                &EntityType::COW,
            )),
            hits: Mutex::default(),
            accepted: true,
        });
        projectile.on_hit(ProjectileHit::Entity {
            entity: target.clone(),
            hit_pos: Vector3::default(),
            normal: Vector3::default(),
        });
        assert_eq!(target.hits.lock().unwrap()[0].amount, 6.0);
        assert_eq!(
            target.get_entity().velocity.load(),
            Vector3::new(0.0, 0.1, 1.2)
        );
    }
}

#[tokio::test]
async fn arrow_damage_enchantments_use_the_actual_hit_target() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    // A command-created weapon exercises Impaling's real entity-type tag requirement.
    let mut bow = ItemStack::new(1, &Item::BOW);
    bow.add_enchantment(&Enchantment::IMPALING, 1);
    for (kind, damage) in [(&EntityType::COW, 4.0), (&EntityType::SQUID, 9.0)] {
        let arrow = arrow::ArrowEntity::new(
            Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
            None,
        );
        *arrow.weapon.write().unwrap() = Some(bow.clone());
        arrow.entity.velocity.store(Vector3::new(0.0, 0.0, 2.0));
        let target = Arc::new(test_support::Receiver {
            living: LivingEntity::new(Entity::new(world.clone(), Vector3::default(), kind)),
            hits: Mutex::default(),
            accepted: true,
        });
        arrow.on_hit(ProjectileHit::Entity {
            entity: target.clone(),
            hit_pos: Vector3::default(),
            normal: Vector3::default(),
        });
        assert_eq!(target.hits.lock().unwrap()[0].amount, damage);
    }
}

#[tokio::test]
async fn arrow_pickup_waits_for_shaking_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let player = TestPlayer::new(&world).player;
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::ARROW),
        None,
    );
    arrow.pickup.store(arrow::ArrowPickup::Allowed);
    arrow.in_ground.store(true, Ordering::Relaxed);
    arrow.shake_time.store(1, Ordering::Relaxed);
    arrow.on_player_collision(&player);
    assert!(!arrow.entity.is_removed());
    arrow.shake_time.store(0, Ordering::Relaxed);
    arrow.on_player_collision(&player);
    assert!(arrow.entity.is_removed());
}

#[tokio::test]
async fn trident_pickup_waits_and_restricts_resolved_owner() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = TestPlayer::new(&world).player;
    let other = TestPlayer::new(&world).player;
    world
        .players
        .store(Arc::new(vec![owner.clone(), other.clone()]));
    let trident = trident::TridentEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::TRIDENT),
        None,
    );
    trident.projectile.set_owner(Some(owner.get_entity()));
    trident.pickup.store(arrow::ArrowPickup::Allowed);
    trident.in_ground.store(true, Ordering::Relaxed);
    trident.shake_time.store(1, Ordering::Relaxed);
    trident.on_player_collision(&owner);
    assert!(!trident.entity.is_removed());
    trident.shake_time.store(0, Ordering::Relaxed);
    trident.on_player_collision(&other);
    assert!(!trident.entity.is_removed());
    trident.on_player_collision(&owner);
    assert!(trident.entity.is_removed());
    let unresolved = trident::TridentEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::TRIDENT),
        None,
    );
    unresolved
        .projectile
        .set_owner_uuid(Some(uuid::Uuid::new_v4()));
    unresolved.pickup.store(arrow::ArrowPickup::Allowed);
    unresolved.in_ground.store(true, Ordering::Relaxed);
    unresolved.on_player_collision(&other);
    assert!(unresolved.entity.is_removed());
}
