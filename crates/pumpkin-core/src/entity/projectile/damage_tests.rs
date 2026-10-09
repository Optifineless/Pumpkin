mod piercing_tests;

use super::*;
use crate::entity::living::test_support::armor_test_world;
use pumpkin_data::{damage::DamageType, effect::StatusEffect};
use std::sync::Mutex;

use super::test_support::{Hit, Receiver};

fn hit(target: &Arc<Receiver>) -> ProjectileHit {
    ProjectileHit::Entity {
        entity: target.clone(),
        hit_pos: target.get_entity().pos.load(),
        normal: Vector3::default(),
    }
}

#[tokio::test]
async fn arrow_and_trident_hit_entry_points_deliver_owner_and_direct_without_raw_position() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let target = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: false,
    });
    target.get_entity().fire_ticks.store(17, Ordering::Relaxed);
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW),
        Some(owner.entity.entity_id),
    );
    arrow.entity.velocity.store(Vector3::new(0.0, 0.0, 2.0));
    arrow.is_flame.store(true, Ordering::Relaxed);
    arrow.on_hit(hit(&target));
    assert_eq!(
        target.hits.lock().unwrap()[0],
        Hit {
            amount: 4.0,
            kind: DamageType::ARROW.id,
            direct: Some(arrow.entity.entity_id),
            cause: Some(owner.entity.entity_id),
            raw_position: None
        }
    );
    assert_eq!(target.get_entity().fire_ticks.load(Ordering::Relaxed), 17);
    assert_eq!(target.get_entity().velocity.load(), Vector3::default());
    let trident = trident::TridentEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::TRIDENT),
        Some(owner.entity.entity_id),
    );
    trident.entity.velocity.store(Vector3::new(2.0, 3.0, 4.0));
    trident.on_hit(hit(&target));
    assert_eq!(
        trident.entity.velocity.load(),
        Vector3::new(-0.02, -0.30000000000000004, -0.04)
    );
    let mut saved = pumpkin_nbt::compound::NbtCompound::new();
    EntityBase::write_nbt(&trident, &mut saved);
    assert_eq!(saved.get_bool("DealtDamage"), Some(true));
    {
        let hits = target.hits.lock().unwrap();
        assert_eq!(hits[1].direct, Some(trident.entity.entity_id));
        assert_eq!(hits[1].cause, Some(owner.entity.entity_id));
        assert_eq!(hits[1].raw_position, None);
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn ownerless_and_nonliving_projectile_branches_match_vanilla_sources() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::ITEM,
    ));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let target = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: false,
    });
    target.get_entity().fire_ticks.store(11, Ordering::Relaxed);
    let small = small_fireball::SmallFireballEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::SMALL_FIREBALL,
    ));
    small.on_hit(hit(&target));
    assert_eq!(
        target.hits.lock().unwrap()[0],
        Hit {
            amount: 5.0,
            kind: DamageType::UNATTRIBUTED_FIREBALL.id,
            direct: Some(small.get_entity().entity_id),
            cause: Some(small.get_entity().entity_id),
            raw_position: None
        }
    );
    assert_eq!(target.get_entity().fire_ticks.load(Ordering::Relaxed), 11);
    let large = fireball::FireballEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::FIREBALL,
    ));
    large.on_hit(hit(&target));
    assert_eq!(
        target.hits.lock().unwrap()[1],
        Hit {
            amount: 6.0,
            kind: DamageType::UNATTRIBUTED_FIREBALL.id,
            direct: Some(large.get_entity().entity_id),
            cause: Some(large.get_entity().entity_id),
            raw_position: None
        }
    );
    assert_eq!(target.get_entity().fire_ticks.load(Ordering::Relaxed), 11);
    let skull = wither_skull::WitherSkullEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::WITHER_SKULL,
    ));
    skull.thrown.projectile.set_owner(Some(&owner));
    skull.on_hit(hit(&target));
    assert_eq!(
        target.hits.lock().unwrap()[2],
        Hit {
            amount: 5.0,
            kind: DamageType::MAGIC.id,
            direct: None,
            cause: None,
            raw_position: None
        }
    );
    assert!(!target.living.has_effect(&StatusEffect::WITHER));
    let spit = llama_spit::LlamaSpitEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::LLAMA_SPIT,
    ));
    spit.on_hit(hit(&target));
    spit.thrown.projectile.set_owner(Some(&owner));
    spit.on_hit(hit(&target));
    assert_eq!(target.hits.lock().unwrap().len(), 3);
    let pearl = ender_pearl::EnderPearlEntity::new(Entity::new(
        world,
        Vector3::default(),
        &EntityType::ENDER_PEARL,
    ));
    pearl.on_hit(hit(&target));
    assert_eq!(
        target.hits.lock().unwrap()[3],
        Hit {
            amount: 0.0,
            kind: DamageType::THROWN.id,
            direct: Some(pearl.get_entity().entity_id),
            cause: None,
            raw_position: None
        }
    );
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_arrow_damage_serializes_the_victim_owner_and_projectile_in_order() {
    use crate::net::java::combat_test_support::TestPlayer;
    use crate::server::combat_test_support::{server, world};
    use pumpkin_protocol::ser::NetworkReadExt;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = TestPlayer::new(&world);
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    let arrow = arrow::ArrowEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::ARROW),
        Some(owner.entity.entity_id),
    );
    arrow.entity.velocity.store(Vector3::new(0.0, 0.0, 2.0));
    fixture.take_packets();
    arrow.on_hit(ProjectileHit::Entity {
        entity: fixture.player.clone(),
        hit_pos: fixture.player.get_entity().pos.load(),
        normal: Vector3::default(),
    });
    let mut found = false;
    for bytes in fixture.take_packets() {
        let mut data = bytes.as_ref();
        if data.get_var_int().unwrap().0 == pumpkin_data::packet::clientbound::play::DAMAGE_EVENT.0
        {
            assert_eq!(
                data.get_var_int().unwrap().0,
                fixture.player.get_entity().entity_id
            );
            assert_eq!(
                data.get_var_int().unwrap().0,
                i32::from(DamageType::ARROW.id)
            );
            assert_eq!(data.get_var_int().unwrap().0, owner.entity.entity_id + 1);
            assert_eq!(data.get_var_int().unwrap().0, arrow.entity.entity_id + 1);
            assert_eq!(data, &[0]);
            found = true;
        }
    }
    assert!(found);
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn piercing_arrows_bypass_a_raised_shield_through_the_hit_entry_point() {
    use crate::net::java::combat_test_support::TestPlayer;
    use crate::server::combat_test_support::{server, world};
    use pumpkin_data::item::Item;
    use pumpkin_util::Hand;
    for piercing in [0, 1] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let fixture = TestPlayer::new(&world);
        let player = &fixture.player;
        let shield = ItemStack::new(1, &Item::SHIELD);
        player.inventory.set_slot(0, shield.clone());
        player.living_entity.set_active_hand(
            Hand::Right,
            shield.clone(),
            shield.get_max_use_time() - 5,
        );
        let arrow = arrow::ArrowEntity::new(
            Entity::new(world, Vector3::new(0.0, 100.0, 1.0), &EntityType::ARROW),
            None,
        );
        arrow.entity.velocity.store(Vector3::new(0.0, 0.0, -2.0));
        arrow.pierce_level.store(piercing, Ordering::Relaxed);
        arrow.on_hit(ProjectileHit::Entity {
            entity: player.clone(),
            hit_pos: player.get_entity().pos.load(),
            normal: Vector3::default(),
        });
        assert_eq!(
            player.living_entity.health.load(),
            if piercing == 0 { 20.0 } else { 16.0 }
        );
        assert_eq!(
            player.inventory.held_item().get_damage(),
            if piercing == 0 { 5 } else { 0 }
        );
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mob_damage_hooks_distinguish_projectiles_from_their_causing_entity() {
    use crate::entity::{boss::wither::WitherEntity, mob::witch::WitchEntity};
    use crate::server::combat_test_support::{server, world};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let witch = WitchEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::WITCH,
    ));
    let direct = Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW);
    assert!(!witch.damage_with_context(
        witch.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        Some(&direct),
        Some(witch.as_ref())
    ));
    let wither = WitherEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::WITHER,
    ));
    let friend = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::SKELETON,
    ));
    assert!(!wither.damage_with_context(
        wither.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        Some(&direct),
        Some(&friend)
    ));
    let cow = LivingEntity::new(Entity::new(world, Vector3::default(), &EntityType::COW));
    wither.get_living_entity().unwrap().set_health(10.0);
    assert!(!wither.damage_with_context(
        wither.as_ref(),
        4.0,
        DamageType::ARROW,
        None,
        Some(&direct),
        Some(&cow)
    ));
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn wind_charge_and_shulker_bullet_filter_nonliving_owners_at_their_entry_points() {
    use crate::entity::mob::shulker::Axis;
    use crate::server::combat_test_support::{server, world};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = Arc::new(Entity::new(
        world.clone(),
        Vector3::new(5.0, 80.0, 5.0),
        &EntityType::ITEM,
    ));
    let target = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(8.0, 80.0, 8.0),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: false,
    });
    world
        .entities
        .store(Arc::new(vec![owner.clone(), target.clone()]));
    let wind = wind_charge::WindChargeEntity::new_normal(ThrownItemEntity::new(
        Entity::new(world, Vector3::default(), &EntityType::WIND_CHARGE),
        &owner,
        0.0,
    ));
    wind.on_hit(hit(&target));
    assert_eq!(target.hits.lock().unwrap()[0].cause, None);
    let bullet = shulker_bullet::ShulkerBulletEntity::new(
        &owner,
        target.get_entity().entity_id,
        target.get_entity().pos.load(),
        Axis::Y,
    );
    bullet.get_entity().set_pos(target.get_entity().pos.load());
    bullet.tick(&bullet, &server);
    {
        let hits = target.hits.lock().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].kind, DamageType::MOB_PROJECTILE.id);
        assert_eq!(hits[1].direct, Some(bullet.get_entity().entity_id));
        assert_eq!(hits[1].cause, None);
        assert_eq!(hits[1].raw_position, None);
    };
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn living_skull_owners_receive_eight_damage_context_and_only_success_applies_wither() {
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let owner = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    )));
    world.entities.store(Arc::new(vec![owner.clone()]));
    for accepted in [false, true] {
        let target = Arc::new(Receiver {
            living: LivingEntity::new(Entity::new(
                world.clone(),
                Vector3::default(),
                &EntityType::COW,
            )),
            hits: Mutex::default(),
            accepted,
        });
        let skull = wither_skull::WitherSkullEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::WITHER_SKULL,
        ));
        skull.thrown.projectile.set_owner(Some(&owner.entity));
        skull.on_hit(hit(&target));
        assert_eq!(
            target.hits.lock().unwrap()[0],
            Hit {
                amount: 8.0,
                kind: DamageType::WITHER_SKULL.id,
                direct: Some(skull.get_entity().entity_id),
                cause: Some(owner.entity.entity_id),
                raw_position: None
            }
        );
        assert_eq!(target.living.has_effect(&StatusEffect::WITHER), accepted);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_projectile_owners_break_vehicles_and_fixed_item_frames() {
    use crate::entity::{
        decoration::item_frame::ItemFrameEntity,
        vehicle::{boat::BoatEntity, minecart::MinecartEntity},
    };
    use crate::net::java::combat_test_support::TestPlayer;
    use crate::server::combat_test_support::{server, world};
    use pumpkin_util::GameMode;
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let owner = fixture.player.as_ref();
    let direct = Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW);
    for creative in [false, true] {
        owner.gamemode.store(if creative {
            GameMode::Creative
        } else {
            GameMode::Survival
        });
        let boat = BoatEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::OAK_BOAT,
        ));
        let minecart = MinecartEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::MINECART,
        ));
        let frame = ItemFrameEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::ITEM_FRAME,
        ));
        frame.set_fixed(true);
        for target in [&boat as &dyn EntityBase, &minecart, &frame] {
            target.damage_with_context(
                target,
                1.0,
                DamageType::ARROW,
                None,
                Some(&direct),
                Some(owner),
            );
            assert_eq!(target.get_entity().is_removed(), creative);
        }
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tnt_minecart_ignition_uses_the_direct_arrows_fire_state() {
    use crate::entity::vehicle::minecart::MinecartEntity;
    use crate::server::combat_test_support::{server, world};
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let owner = LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    ));
    for arrow_burning in [false, true] {
        let minecart = MinecartEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::TNT_MINECART,
        ));
        let direct = Entity::new(world.clone(), Vector3::default(), &EntityType::ARROW);
        if arrow_burning {
            direct.set_on_fire_for(5.0);
        } else {
            minecart.get_entity().set_on_fire_for(5.0);
        }
        minecart.damage_with_context(
            &minecart,
            1.0,
            DamageType::ARROW,
            None,
            Some(&direct),
            Some(&owner),
        );
        assert_eq!(minecart.get_entity().is_removed(), arrow_burning);
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test]
async fn attached_firework_uses_saved_component_damage_and_projectile_context() {
    use pumpkin_data::data_component_impl::{
        FireworkExplosionImpl, FireworkExplosionShape, FireworksImpl,
    };
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let target = Arc::new(Receiver {
        living: LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::COW,
        )),
        hits: Mutex::default(),
        accepted: true,
    });
    world.entities.store(Arc::new(vec![target.clone()]));
    let mut stack = ItemStack::new(1, &pumpkin_data::item::Item::FIREWORK_ROCKET);
    stack.set_data_component(FireworksImpl::new(
        3,
        vec![FireworkExplosionImpl::new(
            FireworkExplosionShape::SmallBall,
            vec![0xff0000],
            vec![],
            false,
            false,
        )],
    ));
    let rocket = firework_rocket::FireworkRocketEntity::with_item(
        Entity::new(
            world.clone(),
            Vector3::default(),
            &EntityType::FIREWORK_ROCKET,
        ),
        stack,
        Some(target.get_entity()),
        true,
    );
    let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
    EntityBase::write_nbt(&rocket, &mut nbt);
    assert!((40..=51).contains(&nbt.get_int("LifeTime").unwrap()));
    assert!(nbt.get_compound("FireworksItem").is_some());
    rocket.explode_and_remove(&world);
    assert_eq!(
        *target.hits.lock().unwrap(),
        vec![Hit {
            amount: 7.0,
            kind: DamageType::FIREWORKS.id,
            direct: Some(rocket.get_entity().entity_id),
            cause: Some(target.get_entity().entity_id),
            raw_position: None,
        }]
    );
    assert!(rocket.get_entity().is_removed());
    crate::server::fixture_lifecycle::finish().await;
}
