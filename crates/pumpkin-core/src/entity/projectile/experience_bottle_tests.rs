use super::*;
use crate::{
    entity::{death_test_world::DeathTestWorld, experience_orb::ExperienceOrbEntity},
    item::{ItemBehaviour, items::experience_bottle::ExperienceBottleItem},
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::{Hand, math::vector3::Vector3};
use std::sync::{Arc, atomic::Ordering};

fn total_xp(world: &crate::world::World) -> i32 {
    world
        .entities
        .load()
        .iter()
        .filter_map(|entity| {
            let orb = entity.cast_any().downcast_ref::<ExperienceOrbEntity>()?;
            let mut nbt = NbtCompound::new();
            orb.write_custom_nbt(&mut nbt);
            Some(orb.get_value() * nbt.get_int("Count").unwrap())
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experience_bottle_awards_only_on_impact_and_discards() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let bottle = Arc::new(ExperienceBottleEntity::new(Entity::new(
        world.clone(),
        Vector3::new(8.5, 65.0, 8.5),
        &EntityType::EXPERIENCE_BOTTLE,
    )));
    bottle
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, -0.5, 0.0));
    assert!(world.spawn_entity(bottle.clone()));
    assert_eq!(total_xp(&world), 0);
    for _ in 0..20 {
        bottle.tick(bottle.as_ref(), &fixture.server);
        if bottle.get_entity().is_removed() {
            break;
        }
        assert_eq!(total_xp(&world), 0);
    }
    assert!(bottle.get_entity().is_removed());
    let amount = total_xp(&world);
    assert!((3..=11).contains(&amount));
    bottle.tick(bottle.as_ref(), &fixture.server);
    assert_eq!(total_xp(&world), amount);
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experience_bottle_offhand_launch_uses_shooter_origin_and_movement() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let player = fixture.player("OffhandBottles");
    player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    player.get_entity().on_ground.store(true, Ordering::Relaxed);
    player.known_movement.record(Vector3::new(6.0, 4.0, 0.0));
    for (hand, count) in [(Hand::Right, 3), (Hand::Left, 5)] {
        player
            .inventory()
            .set_stack_in_hand(hand, ItemStack::new(count, &Item::EXPERIENCE_BOTTLE));
    }
    ExperienceBottleItem.normal_use_with_hand(
        &Item::EXPERIENCE_BOTTLE,
        &player,
        0.0,
        0.0,
        Hand::Left,
    );
    assert_eq!(
        player.inventory().get_stack_in_hand(Hand::Right).item_count,
        3
    );
    assert_eq!(
        player.inventory().get_stack_in_hand(Hand::Left).item_count,
        4
    );
    let entities = world.entities.load_full();
    let bottle = entities
        .iter()
        .find_map(|entity| entity.cast_any().downcast_ref::<ExperienceBottleEntity>())
        .unwrap();
    let entity = bottle.get_entity();
    let origin = entity.pos.load();
    assert!((origin.y - player.eye_position().y + 0.1).abs() < 0.001);
    assert_eq!(
        entity.block_pos.load(),
        pumpkin_util::math::position::BlockPos::floored_v(origin)
    );
    assert!((entity.bounding_box.load().min.y - origin.y).abs() < 0.001);
    assert!(entity.velocity.load().x > 5.0);
    assert!(entity.velocity.load().y < 1.0);
    assert_eq!(total_xp(&world), 0);
    fixture.server.shutdown().await;
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn experience_bottle_item_survives_nbt_reload() {
    let fixture = crate::world::spawn_test_support::Fixture::new();
    let create = || {
        ExperienceBottleEntity::new(Entity::new(
            fixture.world.clone(),
            Vector3::default(),
            &EntityType::EXPERIENCE_BOTTLE,
        ))
    };
    let original = create();
    let mut item = ItemStack::new(7, &Item::EXPERIENCE_BOTTLE);
    item.set_data_component(pumpkin_data::data_component_impl::CustomNameImpl {
        name: pumpkin_util::text::TextComponent::text("Stored bottle"),
    });
    original.set_item_stack(&item);
    let mut nbt = NbtCompound::new();
    original.write_custom_nbt(&mut nbt);
    let restored = create();
    restored.read_custom_nbt(&nbt);
    assert!(
        restored
            .item_stack
            .read()
            .unwrap()
            .are_equal(&item.copy_with_count(1))
    );
    fixture.finish().await;
    crate::server::fixture_lifecycle::finish().await;
}
