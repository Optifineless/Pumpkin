//! External review reproductions through real potion, damage and packet handlers.
use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{ProjectileHit, splash_potion::SplashPotionEntity},
    },
    net::java::combat_test_support::TestPlayer,
    server::combat_test_support,
    world::spawn_test_support,
};
use pumpkin_data::{
    Block,
    biome::Biome,
    damage::DamageType,
    data_component_impl::PotionContentsImpl,
    effect::StatusEffect,
    entity::EntityType,
    item::Item,
    item_stack::ItemStack,
    potion::{Effect, Potion},
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    codec::var_int::VarInt,
    java::server::play::{SInteract, SUseItem},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::{sync::Arc, sync::atomic::Ordering::Relaxed};

fn splash(world: &Arc<crate::world::World>, position: Vector3<f64>, potion: &Potion) {
    let thrown = SplashPotionEntity::new(Entity::new(
        world.clone(),
        position,
        &EntityType::SPLASH_POTION,
    ));
    let mut stack = ItemStack::new(1, &Item::SPLASH_POTION);
    stack.set_data_component(PotionContentsImpl {
        potion_id: Some(i32::from(potion.id)),
        custom_color: None,
        custom_effects: Vec::new(),
        custom_name: None,
    });
    thrown.set_item_stack(stack);
    thrown.on_hit(ProjectileHit::Block {
        pos: position.to_block_pos(),
        world_border: false,
        face: pumpkin_data::BlockDirection::Up,
        hit_pos: position,
        normal: Vector3::new(0.0, 1.0, 0.0),
    });
}

#[ignore = "external-review reproduction R10; passes once combat task 1 lands"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r10_zero_heal_is_a_noop() {
    // LivingEntity.heal accepts zero, including rounded HealOrHarmMobEffect potency.
    let dir = tempfile::tempdir().unwrap();
    let world = super::test_support::armor_test_world(dir.path());
    let living = super::LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    ));
    living.set_health(3.0);
    living.heal(0.0);
    assert_eq!(living.health.load(), 3.0);
    assert!(world.level.shutdown().await.is_ok());
}

#[ignore = "external-review reproduction R10; passes once combat task 1 lands"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r10_healing_splash_at_four_blocks_does_not_panic() {
    // ThrownSplashPotion.onHitAsPotion measures bounding boxes; HealOrHarmMobEffect rounds
    // this newborn projectile's edge potency to zero, which LivingEntity.heal accepts.
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(4.0, 64.0, 0.0));
    fixture.player.living_entity.set_health(5.0);
    splash(&world, Vector3::new(0.0, 64.0, 0.0), &Potion::HEALING);
    assert_eq!(fixture.player.living_entity.health.load(), 5.0);
    assert!(world.level.shutdown().await.is_ok());
}

#[ignore = "external-review reproduction R10; passes once combat task 1 lands"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r10_dead_entity_cannot_be_healed_out_of_death() {
    // LivingEntity.heal only changes health when its current value is positive.
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let victim = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    spawn_test_support::publish(
        &world,
        spawn_test_support::proto(&Biome::PLAINS, &Block::STONE),
    );
    world.add_entity_silent(victim.clone());
    assert!(victim.damage(victim.as_ref(), f32::MAX, DamageType::GENERIC_KILL));
    let living = victim.get_living_entity().unwrap();
    assert!(living.dead.load(Relaxed));
    living.heal(4.0);
    let after_heal = living.health.load();
    for _ in 0..20 {
        living.tick(victim.as_ref(), &server);
    }
    assert_eq!(
        after_heal,
        0.0,
        "heal revived health while dead={}, removed={}, death_time={}",
        living.dead.load(Relaxed),
        victim.get_entity().is_removed(),
        living.death_time.load(Relaxed)
    );
    assert!(victim.get_entity().is_removed());
    assert!(world.level.shutdown().await.is_ok());
}

#[ignore = "external-review reproduction R11; passes once combat task 1 lands"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r11_harming_splash_uses_offhand_totem() {
    // HealOrHarmMobEffect.applyInstantaneousEffect keeps the LivingEntity dynamic type.
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player.get_entity().set_pos(Vector3::new(0.0, 64.0, 0.0));
    player.living_entity.set_health(1.0);
    player
        .inventory()
        .set_stack(40, ItemStack::new(1, &Item::TOTEM_OF_UNDYING));
    assert!(player.damage(player.as_ref(), 12.0, DamageType::MAGIC));
    assert_eq!(player.living_entity.health.load(), 1.0);
    assert!(
        player.inventory().off_hand_item().is_empty(),
        "typed MAGIC control must consume the totem"
    );
    player.living_entity.reset_effects_and_attributes();
    player.living_entity.set_absorption(0.0);
    player.living_entity.hurt_cooldown.store(0, Relaxed);
    player
        .inventory()
        .set_stack(40, ItemStack::new(1, &Item::TOTEM_OF_UNDYING));
    assert_eq!(
        player.inventory().off_hand_item().item,
        &Item::TOTEM_OF_UNDYING
    );
    splash(&world, player.position(), &Potion::STRONG_HARMING);
    assert_eq!(
        (
            player.living_entity.health.load(),
            player.living_entity.dead.load(Relaxed),
            player.inventory().off_hand_item().is_empty()
        ),
        (1.0, false, true)
    );
    assert!(world.level.shutdown().await.is_ok());
}

#[ignore = "external-review reproduction R14; passes once combat task 1 lands"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r14_resistance_six_clamps_damage_to_zero() {
    // LivingEntity.getDamageAfterMagicAbsorb uses signed int arithmetic and Math.max(..., 0).
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let living = super::LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::default(),
        &EntityType::COW,
    ));
    living.add_effect(Effect {
        effect_type: &StatusEffect::RESISTANCE,
        duration: 200,
        amplifier: 5,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    });
    let before = living.health.load();
    assert!(living.damage(&living, 1.0, DamageType::GENERIC));
    assert_eq!(living.health.load(), before);
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r12_offhand_bucket_replaces_only_offhand() {
    // BucketItem.use / ServerPlayerGameMode.useItem return the replacement to the requested hand.
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let mut chunk = spawn_test_support::proto(&Biome::PLAINS, &Block::STONE);
    chunk.set_block_state(8, 65, 11, Block::STONE.default_state);
    spawn_test_support::publish(&world, chunk);
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::DIAMOND_SWORD));
    player
        .inventory()
        .set_stack(40, ItemStack::new(1, &Item::WATER_BUCKET));
    fixture.client().handle_use_item(
        player,
        &SUseItem {
            hand: VarInt(1),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &server,
    );
    assert_eq!(
        world.get_block(&BlockPos::new(8, 65, 10)),
        &Block::WATER,
        "fixture must actually empty the bucket"
    );
    assert_eq!(
        (
            player.inventory().held_item().item,
            player.inventory().off_hand_item().item
        ),
        (&Item::DIAMOND_SWORD, &Item::BUCKET)
    );
    assert!(world.level.shutdown().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ext_review_r13_fish_capture_keeps_filled_bucket_after_packet_writeback() {
    // Bucketable.bucketMobPickup sets the filled result in the hand before discarding the fish.
    let dir = tempfile::tempdir().unwrap();
    let server = combat_test_support::server(dir.path());
    let world = combat_test_support::world(&server, dir.path());
    let fixture = TestPlayer::new(&world);
    let player = &fixture.player;
    player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::WATER_BUCKET));
    let fish = crate::entity::r#type::from_type(
        &EntityType::COD,
        player.position(),
        &world,
        uuid::Uuid::new_v4(),
    );
    world.add_entity_silent(fish.clone());
    fixture.client().handle_interact(
        player,
        &SInteract {
            entity_id: VarInt(fish.get_entity().entity_id),
            r#type: VarInt(2),
            target_position: Some(Vector3::default()),
            hand: Some(VarInt(0)),
            sneaking: false,
        },
        &server,
    );
    assert!(
        fish.get_entity().is_removed(),
        "fixture must reach the fish capture path"
    );
    assert_eq!(player.inventory().held_item().item, &Item::COD_BUCKET);
    assert!(world.level.shutdown().await.is_ok());
}
