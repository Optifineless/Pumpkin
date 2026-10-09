use super::*;
use crate::{
    entity::Entity,
    world::spawn_test_support::{Fixture, proto, publish},
};
use pumpkin_data::{Block, BlockState, biome::Biome, block_properties::OakFenceLikeProperties};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rand::{SeedableRng, rngs::StdRng};
use std::sync::Arc;

fn village(count: usize) -> (Fixture, Vec<Arc<VillagerEntity>>) {
    let fixture = Fixture::new();
    for x in -1..=1 {
        for z in -1..=1 {
            let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
            terrain.x = x;
            terrain.z = z;
            // A house with a low solid roof: neither the floor nor a wall may contain the golem.
            if x == 0 && z == 0 {
                for bx in 5..=10 {
                    for bz in 5..=10 {
                        terrain.set_block_state(bx, 66, bz, Block::STONE.default_state);
                        if bx == 5 || bx == 10 || bz == 5 || bz == 10 {
                            for y in 64..66 {
                                terrain.set_block_state(bx, y, bz, Block::STONE.default_state);
                            }
                        }
                    }
                }
            }
            publish(&fixture.world, terrain);
        }
    }
    fixture.world.level_time.lock().unwrap().world_age = 1_200;
    fixture.world.level_time.lock().unwrap().time_of_day = 1_200;
    let villagers = (0..count)
        .map(|i| {
            let villager = VillagerEntity::new(Entity::new(
                fixture.world.clone(),
                Vector3::new(6.5 + (i % 3) as f64, 64.0, 6.5 + (i / 3) as f64),
                &EntityType::VILLAGER,
            ));
            villager.record_last_slept(1_000);
            villager.last_worked_at_poi.store(1_000, Relaxed);
            *villager.home_pos.lock().unwrap() = Some(BlockPos::new(6, 64, 6));
            fixture.world.add_entity_silent(villager.clone());
            villager
        })
        .collect();
    (fixture, villagers)
}

fn golems(world: &crate::world::World) -> Vec<Arc<dyn EntityBase>> {
    world
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::IRON_GOLEM)
        .cloned()
        .collect()
}

#[tokio::test]
async fn recently_detected_golem_twenty_blocks_away_prevents_house_summons() {
    let (fixture, villagers) = village(3);
    let golem = IronGolemEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::new(14.5, 64.0, 6.5),
        &EntityType::IRON_GOLEM,
    ));
    fixture.world.add_entity_silent(golem.clone());
    for villager in &villagers {
        villager.check_for_nearby_golem(&villager.nearby_entities());
    }
    // The default FOLLOW_RANGE is 16; remember it nearby, then move it twenty blocks away.
    golem.get_entity().set_pos(Vector3::new(26.5, 64.0, 6.5));
    for villager in &villagers {
        villager.spawn_golem_if_needed(1_200, 3, &mut StdRng::seed_from_u64(10));
    }
    assert_eq!(golems(&fixture.world).len(), 1);
    // Detection persists even after the existing golem dies.
    golem.mob_entity.living_entity.health.store(0.0);
    for villager in &villagers {
        assert!(!villager.wants_to_spawn_golem(1_200));
        villager.spawn_golem_if_needed(1_200, 3, &mut StdRng::seed_from_u64(10));
    }
    assert_eq!(golems(&fixture.world).len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn five_qualifying_villagers_summon_one_golem_on_a_clear_collider_top() {
    let (fixture, villagers) = village(5);
    for villager in &villagers {
        villager.spawn_golem_if_needed(1_200, 5, &mut StdRng::seed_from_u64(1));
    }
    let summoned = golems(&fixture.world);
    assert_eq!(summoned.len(), 1);
    let golem = summoned[0].get_entity();
    let pos = golem.block_pos.load();
    assert!(
        fixture
            .world
            .get_block_state(&pos.down())
            .is_side_solid(pumpkin_data::BlockDirection::Up)
    );
    assert!(fixture.world.is_space_empty(golem.bounding_box.load()));
    assert!(
        fixture
            .world
            .get_block_collisions(golem.bounding_box.load(), summoned[0].as_ref())
            .0
            .is_empty()
    );
    assert!(golem.pos.load().y == 64.0 || golem.pos.load().y == 67.0);
    fixture.finish().await;
}

#[tokio::test]
async fn golem_placement_rejects_a_fence_protruding_from_below_its_feet() {
    let fixture = Fixture::new();
    let mut terrain = proto(&Biome::PLAINS, &Block::STONE);
    let mut fence = OakFenceLikeProperties::default(&Block::OAK_FENCE);
    fence.west = true;
    terrain.set_block_state(
        9,
        63,
        8,
        BlockState::from_id(fence.to_state_id(&Block::OAK_FENCE)),
    );
    publish(&fixture.world, terrain);
    assert!(
        try_spawn_mob_with_random(
            &EntityType::IRON_GOLEM,
            SpawnReason::MobSummoned,
            IronGolemEntity::new,
            &fixture.world,
            &BlockPos::new(8, 64, 8),
            1,
            0,
            0,
            SpawnStrategy::OnTopOfColliderNoLeaves,
            true,
            &mut StdRng::seed_from_u64(1),
        )
        .is_none()
    );
    assert!(golems(&fixture.world).is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn summon_marks_every_nearby_villager_and_blocks_the_next_panic_check() {
    let (fixture, villagers) = village(6);
    // An unqualified villager also gets the memory after its neighbors summon.
    villagers[5]
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .erase(types::LAST_SLEPT.id());
    villagers[0].spawn_golem_if_needed(1_200, 5, &mut StdRng::seed_from_u64(2));
    assert_eq!(golems(&fixture.world).len(), 1);
    for golem in golems(&fixture.world) {
        golem.get_living_entity().unwrap().health.store(0.0);
    }
    for villager in &villagers {
        assert_eq!(
            villager
                .mob_entity
                .brain
                .lock()
                .unwrap()
                .get(types::GOLEM_DETECTED_RECENTLY),
            Some(&true)
        );
        for _ in 0..100 {
            villager.mob_entity.tick_brain(villager.as_ref());
        }
        villager.spawn_golem_if_needed(1_300, 3, &mut StdRng::seed_from_u64(3));
    }
    assert_eq!(golems(&fixture.world).len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn golem_eligibility_requires_sleep_and_memory_expires_after_six_hundred_ticks() {
    let (fixture, villagers) = village(5);
    let villager = &villagers[0];
    villager
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .erase(types::LAST_SLEPT.id());
    assert!(!villager.wants_to_spawn_golem(1_200));
    villager.record_last_slept(0);
    villager.last_worked_at_poi.store(0, Relaxed);
    assert!(villager.wants_to_spawn_golem(23_999));
    assert!(!villager.wants_to_spawn_golem(24_000));
    villager.golem_detected();
    for _ in 0..599 {
        villager.mob_entity.tick_brain(villager.as_ref());
    }
    assert!(!villager.wants_to_spawn_golem(1_200));
    villager.mob_entity.tick_brain(villager.as_ref());
    assert!(villager.wants_to_spawn_golem(1_200));
    let saved = villager.mob_entity.brain.lock().unwrap().pack();
    assert_eq!(make_brain(&saved).get(types::LAST_SLEPT), Some(&0));
    fixture.finish().await;
}

#[tokio::test]
async fn panic_and_gossip_use_three_and_five_willing_villagers() {
    let (fixture, villagers) = village(3);
    let caller = &villagers[0];
    caller.gossip_golem_check(
        1_200,
        &caller.nearby_entities(),
        &mut StdRng::seed_from_u64(5),
    );
    assert!(golems(&fixture.world).is_empty());
    villagers[2]
        .mob_entity
        .brain
        .lock()
        .unwrap()
        .erase(types::LAST_SLEPT.id());
    caller.spawn_golem_if_needed(1_200, 3, &mut StdRng::seed_from_u64(5));
    assert!(golems(&fixture.world).is_empty());
    villagers[2].record_last_slept(1_000);
    assert!(caller.damage(
        caller.as_ref(),
        1.0,
        pumpkin_data::damage::DamageType::GENERIC
    ));
    // Exercise the real VillagerPanicTrigger adapter, including its 100 game-tick cadence.
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(5));
    assert_eq!(golems(&fixture.world).len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn concurrent_villagers_summon_exactly_one_golem() {
    let (fixture, villagers) = village(5);
    let barrier = std::sync::Barrier::new(villagers.len());
    std::thread::scope(|scope| {
        for villager in &villagers {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                villager.spawn_golem_if_needed(1_200, 5, &mut StdRng::seed_from_u64(7));
            });
        }
    });
    assert_eq!(golems(&fixture.world).len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn five_idle_villagers_summon_through_social_ai_step() {
    let (fixture, villagers) = village(5);
    let caller = &villagers[0];
    assert_eq!(caller.resolved_golem_activity(), Activity::Idle);
    assert!(!caller.is_panicking(&caller.nearby_entities()));
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
    assert_eq!(golems(&fixture.world).len(), 1);
    assert!(
        caller
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id())
    );
    assert_eq!(caller.last_gossip_share_time.load(Relaxed), 1_200);
    fixture.finish().await;
}

#[tokio::test]
async fn five_working_villagers_do_not_gossip_summon() {
    let (fixture, villagers) = village(5);
    fixture.world.level_time.lock().unwrap().time_of_day = 3_000;
    let caller = &villagers[0];
    *caller.job_site.lock().unwrap() = Some(BlockPos::new(6, 64, 6));
    // A partner remembered in IDLE must also be cleared when WORK becomes active.
    caller.mob_entity.brain.lock().unwrap().set(
        types::INTERACTION_TARGET,
        villagers[1].clone() as Arc<dyn EntityBase>,
    );
    assert_eq!(caller.resolved_golem_activity(), Activity::Work);
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
    assert!(golems(&fixture.world).is_empty());
    assert_eq!(caller.last_gossip_share_time.load(Relaxed), 0);
    assert!(
        !caller
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id())
    );
    fixture.finish().await;
}

#[tokio::test]
async fn social_ai_uses_idle_when_scheduled_work_or_meet_lacks_its_poi() {
    for daytime in [3_000, 9_500] {
        let (fixture, villagers) = village(5);
        fixture.world.level_time.lock().unwrap().time_of_day = daytime;
        let caller = &villagers[0];
        assert_eq!(caller.resolved_golem_activity(), Activity::Idle);
        caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
        assert_eq!(golems(&fixture.world).len(), 1);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn five_meeting_villagers_summon_through_social_ai_step() {
    use crate::entity::ai::brain::memory::GlobalPos;
    let (fixture, villagers) = village(5);
    fixture.world.level_time.lock().unwrap().time_of_day = 9_500;
    let caller = &villagers[0];
    caller.mob_entity.brain.lock().unwrap().set(
        types::MEETING_POINT,
        GlobalPos::new(
            &pumpkin_data::dimension::Dimension::OVERWORLD,
            BlockPos::new(8, 64, 8),
        ),
    );
    assert_eq!(caller.resolved_golem_activity(), Activity::Meet);
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
    assert_eq!(golems(&fixture.world).len(), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn social_ai_needs_an_interaction_partner_in_gossip_range() {
    let (fixture, villagers) = village(5);
    let caller = &villagers[0];
    caller.get_entity().set_pos(Vector3::new(6.5, 67.0, 6.5));
    for (index, villager) in villagers.iter().enumerate().skip(1) {
        villager
            .get_entity()
            .set_pos(Vector3::new(10.5 + index as f64, 67.0, 6.5));
    }
    // Five willing villagers are nearby, but their interaction partner is too far to gossip.
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
    assert!(golems(&fixture.world).is_empty());
    assert!(
        caller
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id())
    );
    for villager in villagers.iter().skip(1) {
        villager.get_entity().set_pos(Vector3::new(40.5, 64.0, 8.5));
    }
    caller.golem_ai_step_with_random(&mut StdRng::seed_from_u64(1));
    assert!(
        !caller
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .has_memory_value(types::INTERACTION_TARGET.id())
    );
    assert_eq!(caller.last_gossip_share_time.load(Relaxed), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn villager_nbt_round_trip_preserves_active_golem_memory_remaining_ttl() {
    use pumpkin_nbt::{Nbt, NbtCompound, deserializer::NbtReadHelperJava};
    let (fixture, villagers) = village(1);
    let villager = &villagers[0];
    villager.golem_detected();
    for _ in 0..123 {
        villager.mob_entity.tick_brain(villager.as_ref());
    }
    let mut saved = NbtCompound::new();
    EntityBase::write_nbt(villager.as_ref(), &mut saved);
    assert_eq!(
        saved
            .get_compound("Brain")
            .unwrap()
            .get_compound("memories")
            .unwrap()
            .get_compound("minecraft:golem_detected_recently")
            .unwrap()
            .get_long("ttl"),
        Some(476)
    );
    let bytes = Nbt::new(String::new(), saved).write();
    let decoded = Nbt::read(&mut NbtReadHelperJava::new(std::io::Cursor::new(
        bytes.as_ref(),
    )))
    .unwrap();
    let loaded = VillagerEntity::new(Entity::new(
        fixture.world.clone(),
        Vector3::default(),
        &EntityType::VILLAGER,
    ));
    EntityBase::read_nbt_non_mut(loaded.as_ref(), &decoded.root_tag);
    assert!(!loaded.wants_to_spawn_golem(1_200));
    assert_eq!(
        loaded
            .mob_entity
            .brain
            .lock()
            .unwrap()
            .get(types::LAST_SLEPT),
        Some(&1_000)
    );
    let mut resaved = NbtCompound::new();
    EntityBase::write_nbt(loaded.as_ref(), &mut resaved);
    assert_eq!(
        resaved
            .get_compound("Brain")
            .unwrap()
            .get_compound("memories")
            .unwrap()
            .get_compound("minecraft:golem_detected_recently")
            .unwrap()
            .get_long("ttl"),
        Some(476)
    );
    for _ in 0..476 {
        loaded.mob_entity.tick_brain(loaded.as_ref());
    }
    assert!(!loaded.wants_to_spawn_golem(1_200));
    loaded.mob_entity.tick_brain(loaded.as_ref());
    assert!(loaded.wants_to_spawn_golem(1_200));
    fixture.finish().await;
}
