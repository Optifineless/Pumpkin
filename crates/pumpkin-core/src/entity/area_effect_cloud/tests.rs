use super::*;

#[test]
fn lingering_cloud_wait_decay_use_and_expiry_follow_server_tick() {
    let mut state = CloudState {
        radius: 3.0,
        duration: 600,
        age: 0,
        wait_time: 10,
        reapplication_delay: 20,
        radius_per_tick: -0.005,
        radius_on_use: -0.5,
        duration_on_use: 0,
        victims: HashMap::new(),
    };
    for _ in 0..9 {
        assert!(state.tick());
    }
    assert_eq!(state.radius, 3.0);
    assert!(state.tick());
    assert!((state.radius - 2.995).abs() < 0.0001);
    assert!(state.on_use());
    assert_eq!(state.duration, 600);
    assert!((state.radius - 2.495).abs() < 0.0001);
    state.radius = 0.502;
    assert!(!state.tick());
    state.radius_per_tick = 0.0;
    state.age = 608;
    assert!(state.tick());
    assert!(!state.tick());
    state.duration = -1;
    assert!(state.tick());
}

#[tokio::test]
async fn cloud_applies_at_full_strength_in_a_half_block_disc_and_refreshes_after_twenty_ticks() {
    use crate::entity::living::{LivingEntity, test_support::armor_test_world};
    use pumpkin_data::{effect::StatusEffect, entity::EntityType};
    let dir = tempfile::tempdir().unwrap();
    let world = armor_test_world(dir.path());
    let edge = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(2.5, 65.0, 0.0),
        &EntityType::COW,
    )));
    let above = Arc::new(LivingEntity::new(Entity::new(
        world.clone(),
        Vector3::new(0.0, 65.6, 0.0),
        &EntityType::COW,
    )));
    world
        .entities
        .store(Arc::new(vec![edge.clone(), above.clone()]));
    let cloud = AreaEffectCloudEntity::create(
        Entity::new(
            world,
            Vector3::new(0.0, 65.0, 0.0),
            &EntityType::AREA_EFFECT_CLOUD,
        ),
        ItemStack::new(1, &Item::POTION),
        vec![(&StatusEffect::SPEED, 100, 0, false, true, true)],
        600,
        3.0,
        20,
        10,
        0.0,
        0,
    );
    cloud.state.lock().unwrap().age = 10;
    cloud.apply_to_entities();
    assert_eq!(edge.get_effect(&StatusEffect::SPEED).unwrap().duration, 100);
    assert!(!above.has_effect(&StatusEffect::SPEED));
    edge.active_effects
        .lock()
        .unwrap()
        .get_mut(&StatusEffect::SPEED)
        .unwrap()
        .duration = 80;
    cloud.state.lock().unwrap().age = 29;
    cloud.apply_to_entities();
    assert_eq!(edge.get_effect(&StatusEffect::SPEED).unwrap().duration, 80);
    cloud.state.lock().unwrap().age = 30;
    cloud.apply_to_entities();
    assert_eq!(edge.get_effect(&StatusEffect::SPEED).unwrap().duration, 100);
    let mut stored = pumpkin_nbt::compound::NbtCompound::new();
    EntityBase::write_nbt(cloud.as_ref(), &mut stored);
    EntityBase::read_nbt_non_mut(cloud.as_ref(), &stored);
    assert_eq!(cloud.radius(), 3.0);
    assert!(cloud.state.lock().unwrap().victims.is_empty());
    assert_eq!(cloud.effects.lock().unwrap()[0].1, 100);
}
